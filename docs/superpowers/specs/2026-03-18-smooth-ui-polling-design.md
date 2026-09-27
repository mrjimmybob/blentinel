# Smooth UI Polling — Design Spec

**Date:** 2026-03-18
**File:** `hub/src/app.rs`
**Status:** ~~Approved~~ **Superseded.** The implementation moved to `RwSignal` + `Memo` + `spawn_local` instead of `<Transition>`. The structural fix (SSE push) is tracked in `docs/DECISIONS.md` D-12 and `docs/proposals/0007-history-reporting-storage.md` §3.5.

---

## Problem

The dashboard (`DashboardPage`) and company detail page (`CompanyDetailPage`) flash every 15 seconds. A `set_interval` calls `refetch()` on `LocalResource` values. When `refetch()` fires, Leptos transitions the resource back to a pending/`None` state. `<Suspense>` responds by showing its fallback (`"Loading…"`), blanking the entire view until the fetch completes.

Additionally, the intervals are never cleared: `cb.forget()` leaks the closure and no `clear_interval` is ever called. Navigating away and back registers a new interval on top of the existing one.

---

## Solution

### 1. Replace `<Suspense>` with `<Transition>` in the two polling pages

`<Transition>` is Leptos's built-in component for this exact scenario:
- Shows its fallback **only on the first render** (when no data exists yet).
- On subsequent refetches, keeps showing the **previous content** until new data arrives.
- Drop-in replacement — same `fallback=` prop, no other view changes.

**Affected locations in `hub/src/app.rs`:**

| Component | Opening tag (approx. line) | Closing tag (approx. line) |
|---|---|---|
| `DashboardPage` | 498 | 573 |
| `CompanyDetailPage` — probes/uptime section | 752 | 821 |

> **Line numbers are for orientation only.** After the first edit, downstream line numbers shift. Locate blocks by component name and the `<Suspense` token — do not rely on line numbers.

The `CompanyDetailPage` block wraps **both** the `uptime` and `probes` resource reads inside a single `<Suspense>` (and after this fix, a single `<Transition>`). Both resources share one transition boundary — the transition holds until both have resolved. This is intentional: the two resources are fetched together on the same 15-second tick, so they settle near-simultaneously. Independent boundaries are not needed.

The `<Suspense>` blocks in `ArchiveViewerPage` (~1414) and `AdminPage` (~1624) are **not** on a polling loop and are left unchanged.

### 2. Fix interval cleanup with `on_cleanup`

The current pattern leaks the interval closure and never cancels the interval.

**Current (broken):**
```rust
Effect::new(move |_| {
    #[cfg(not(feature = "ssr"))]
    {
        let cb = Closure::wrap(Box::new(move || {
            companies.refetch();
        }) as Box<dyn FnMut()>);
        let func = cb.as_ref().unchecked_ref::<leptos::web_sys::js_sys::Function>();
        leptos::web_sys::window()
            .unwrap()
            .set_interval_with_callback_and_timeout_and_arguments_0(func, 15_000)
            .unwrap();
        cb.forget(); // leaked — never cleared
    }
});
```

**Fixed:**
```rust
Effect::new(move |_| {
    #[cfg(not(feature = "ssr"))]
    {
        let cb = Closure::wrap(Box::new(move || {
            companies.refetch();
        }) as Box<dyn FnMut()>);
        let func = cb.as_ref().unchecked_ref::<leptos::web_sys::js_sys::Function>();
        let interval_id = leptos::web_sys::window()
            .unwrap()
            .set_interval_with_callback_and_timeout_and_arguments_0(func, 15_000)
            .unwrap();
        // `cb.forget()` is intentional: the JS callback must remain alive for the
        // duration of the interval. `clear_interval` stops future firings but does
        // NOT reclaim the Closure allocation — it remains leaked to the JS heap.
        // This is the standard wasm-bindgen idiom for long-lived interval callbacks;
        // the cost is one small, bounded allocation per component mount. Do not
        // "fix" this by dropping cb before forget() — that causes a use-after-free in WASM.
        cb.forget();
        // on_cleanup is inside #[cfg(not(feature = "ssr"))] intentionally: these are
        // browser-only intervals. Do not use this pattern in SSR-capable cleanup contexts.
        on_cleanup(move || {
            leptos::web_sys::window()
                .unwrap()
                .clear_interval_with_handle(interval_id);
        });
    }
});
```

`on_cleanup` is called by Leptos when the component owning the Effect is unmounted (e.g., navigating away). This cancels the interval and prevents accumulation across navigations.

**Affected locations:**
- `DashboardPage` polling Effect (~line 478): refetches `companies`
- `CompanyDetailPage` polling Effect (~line 717): refetches `probes` and `uptime`

---

## Scope

| File | Changes |
|---|---|
| `hub/src/app.rs` | 4 tag renames (`Suspense` → `Transition`), 2 Effect blocks updated with `on_cleanup` |

No new files. No API changes. No structural refactoring.

---

## Non-goals

- No WebSocket / SSE implementation
- No change to the 15-second polling interval
- No changes to admin or archive viewer pages
