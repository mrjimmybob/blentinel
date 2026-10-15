//! Windows' management store (WMI), read as the probe itself: what the
//! `smart` check reads the disks' health from, and the `hyperv` check its
//! virtual machines. Part of every Windows; nothing is installed for it.

use std::collections::HashMap;

use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx,
    CoSetProxyBlanket, CoUninitialize, EOAC_NONE, RPC_C_AUTHN_LEVEL_CALL,
    RPC_C_IMP_LEVEL_IMPERSONATE,
};
use windows::Win32::System::Wmi::{
    IWbemLocator, IWbemServices, WBEM_FLAG_FORWARD_ONLY, WBEM_FLAG_RETURN_IMMEDIATELY,
    WBEM_GENERIC_FLAG_TYPE, WbemLocator,
};
use windows::core::{BSTR, HRESULT, HSTRING, PCWSTR, VARIANT};

/// How long one answer of a query may take, in milliseconds.
const ANSWER_WAIT_MS: i32 = 10_000;
/// Windows' own sign-in (NTLM or Kerberos), and no authorisation service:
/// `RPC_C_AUTHN_WINNT`, `RPC_C_AUTHZ_NONE`.
const AUTHN_WINNT: u32 = 10;
const AUTHZ_NONE: u32 = 0;
/// The error of a namespace this machine doesn't have
/// (`WBEM_E_INVALID_NAMESPACE`): the feature it belongs to isn't installed.
pub const NO_SUCH_NAMESPACE: HRESULT = HRESULT(0x8004_100E_u32 as i32);

/// An object's fields as text: a number as its digits, a list as its
/// numbers; a field without a value is left out.
pub type Row = HashMap<&'static str, String>;

/// What `ask` reads, on a thread of its own with COM set up for it (the
/// calls block). `ask` gives back no COM object: they are all dropped
/// before COM is closed. `None`: the thread failed.
pub async fn read<T: Send + 'static>(ask: impl FnOnce() -> T + Send + 'static) -> Option<T> {
    tokio::task::spawn_blocking(|| {
        // SAFETY: COM is set up for this thread before it is used, and
        // closed after, once every COM object made in `ask` is dropped.
        unsafe {
            let started = CoInitializeEx(None, COINIT_MULTITHREADED).is_ok();
            let answer = ask();
            if started {
                CoUninitialize();
            }
            answer
        }
    })
    .await
    .ok()
}

/// A part of the store (`ROOT\Microsoft\Windows\Storage`), open for
/// queries. Opened and used within [`read`].
pub struct Namespace(IWbemServices);

impl Namespace {
    pub fn open(path: &str) -> windows::core::Result<Self> {
        // SAFETY: plain COM calls with arguments that live through them;
        // without COM set up on the thread they fail, no more.
        unsafe {
            let locator: IWbemLocator = CoCreateInstance(&WbemLocator, None, CLSCTX_INPROC_SERVER)?;
            let services = locator.ConnectServer(
                &BSTR::from(path),
                &BSTR::new(),
                &BSTR::new(),
                &BSTR::new(),
                0,
                &BSTR::new(),
                None,
            )?;
            // The queries run as the probe itself.
            CoSetProxyBlanket(
                &services,
                AUTHN_WINNT,
                AUTHZ_NONE,
                PCWSTR::null(),
                RPC_C_AUTHN_LEVEL_CALL,
                RPC_C_IMP_LEVEL_IMPERSONATE,
                None,
                EOAC_NONE,
            )?;
            Ok(Self(services))
        }
    }

    /// Every object of `class`, with these fields.
    pub fn rows(&self, class: &str, fields: &[&'static str]) -> windows::core::Result<Vec<Row>> {
        // SAFETY: as in `open`; every buffer handed over outlives its call.
        unsafe {
            let query = format!("SELECT {} FROM {class}", fields.join(", "));
            let found = self.0.ExecQuery(
                &BSTR::from("WQL"),
                &BSTR::from(query),
                WBEM_GENERIC_FLAG_TYPE(WBEM_FLAG_FORWARD_ONLY.0 | WBEM_FLAG_RETURN_IMMEDIATELY.0),
                None,
            )?;
            let mut rows = Vec::new();
            loop {
                let mut object = [None];
                let mut returned = 0;
                found
                    .Next(ANSWER_WAIT_MS, &mut object, &mut returned)
                    .ok()?;
                let Some(object) = object[0].take().filter(|_| returned == 1) else {
                    break;
                };
                let mut row = Row::new();
                for field in fields {
                    let mut value = VARIANT::default();
                    let read = object.Get(&HSTRING::from(*field), 0, &mut value, None, None);
                    if let Some(text) = read.ok().and_then(|()| BSTR::try_from(&value).ok())
                        && !text.is_empty()
                    {
                        row.insert(*field, text.to_string());
                    }
                }
                rows.push(row);
            }
            Ok(rows)
        }
    }
}
