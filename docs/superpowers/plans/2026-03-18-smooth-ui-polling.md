# Smooth UI Polling Implementation Plan

> **For agentic workers:** REQUIRED: Use superpowers:subagent-driven-development (if subagents available) or superpowers:executing-plans to implement this plan. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Eliminate the 15-second UI flash in `DashboardPage` and `CompanyDetailPage` by replacing `<Suspense>` with `<Transition>`, and fix interval leaks by registering `on_cleanup` cancellation for both polling Effects.

**Architecture:** Two targeted edits to a single file (`hub/src/app.rs`). `<Transition>` replaces `<Suspense>` in the two polling pages — it shows the loading fallback only on first render and keeps stale content visible during refetches. Polling `Effect` blocks are updated to capture the interval handle and register `on_cleanup` to cancel it when the component unmounts.

**Tech Stack:** Rust, Leptos 0.8, wasm-bindgen, web-sys

---

## File Map

| File | What changes |
|---|---|
| `hub/src/app.rs` | Task 1: DashboardPage Effect + Suspense; Task 2: CompanyDetailPage Effect + Suspense |

No new files. No new dependencies.

---

## Chunk 1: Both Tasks

### Task 1: Fix `DashboardPage` — interval cleanup + flash

**Files:**
- Modify: `hub/src/app.rs` — `DashboardPage` component

There are no unit tests for Leptos view rendering in this codebase. Correctness is verified by `cargo check` (no compile errors) and manual browser observation.

- [ ] **Step 1: Update the `DashboardPage` polling Effect**

Locate this block in `DashboardPage` (search for `companies.refetch()` — there is exactly one):

```rust
    Effect::new(move |_| {
        #[cfg(not(feature = "ssr"))]
        {
            use leptos::wasm_bindgen::closure::Closure;
            use leptos::wasm_bindgen::JsCast;
            let cb = Closure::wrap(Box::new(move || {
                companies.refetch();
            }) as Box<dyn FnMut()>);
            let func = cb
                .as_ref()
                .unchecked_ref::<leptos::web_sys::js_sys::Function>();
            leptos::web_sys::window()
                .unwrap()
                .set_interval_with_callback_and_timeout_and_arguments_0(func, 15_000)
                .unwrap();
            cb.forget();
        }
    });
```

Replace it with:

```rust
    Effect::new(move |_| {
        #[cfg(not(feature = "ssr"))]
        {
            use leptos::wasm_bindgen::closure::Closure;
            use leptos::wasm_bindgen::JsCast;
            let cb = Closure::wrap(Box::new(move || {
                companies.refetch();
            }) as Box<dyn FnMut()>);
            let func = cb
                .as_ref()
                .unchecked_ref::<leptos::web_sys::js_sys::Function>();
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

- [ ] **Step 2: Replace `<Suspense>` with `<Transition>` in `DashboardPage`**

Still in `DashboardPage`, find the `view!` block. Locate (search for `<Suspense` within `DashboardPage`):

```rust
        <Suspense fallback=|| view! { <div class="loading">"Loading…"</div> }>
```

Change to:

```rust
        <Transition fallback=|| view! { <div class="loading">"Loading…"</div> }>
```

Then find the matching closing tag immediately before the end of the `DashboardPage` `view!`:

```rust
        </Suspense>
```

Change to:

```rust
        </Transition>
```

- [ ] **Step 3: Verify Task 1 compiles**

```bash
cd D:/MyDocuments/Development/blentinel
cargo check -p hub
```

Expected: no errors. Warnings are acceptable. Fix any errors before continuing.

- [ ] **Step 4: Commit Task 1**

```bash
git add hub/src/app.rs
git commit -m "fix: replace Suspense with Transition in DashboardPage, add interval cleanup"
```

---

### Task 2: Fix `CompanyDetailPage` — interval cleanup + flash

**Files:**
- Modify: `hub/src/app.rs` — `CompanyDetailPage` component

- [ ] **Step 5: Update the `CompanyDetailPage` polling Effect**

Locate this block in `CompanyDetailPage` (search for `probes.refetch()` — there is exactly one):

```rust
    // Poll every 15 seconds so probe status and uptime update live.
    Effect::new(move |_| {
        #[cfg(not(feature = "ssr"))]
        {
            use leptos::wasm_bindgen::closure::Closure;
            use leptos::wasm_bindgen::JsCast;
            let cb = Closure::wrap(Box::new(move || {
                probes.refetch();
                uptime.refetch();
            }) as Box<dyn FnMut()>);
            let func = cb
                .as_ref()
                .unchecked_ref::<leptos::web_sys::js_sys::Function>();
            leptos::web_sys::window()
                .unwrap()
                .set_interval_with_callback_and_timeout_and_arguments_0(func, 15_000)
                .unwrap();
            cb.forget();
        }
    });
```

Replace it with:

```rust
    // Poll every 15 seconds so probe status and uptime update live.
    Effect::new(move |_| {
        #[cfg(not(feature = "ssr"))]
        {
            use leptos::wasm_bindgen::closure::Closure;
            use leptos::wasm_bindgen::JsCast;
            let cb = Closure::wrap(Box::new(move || {
                probes.refetch();
                uptime.refetch();
            }) as Box<dyn FnMut()>);
            let func = cb
                .as_ref()
                .unchecked_ref::<leptos::web_sys::js_sys::Function>();
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

- [ ] **Step 6: Replace `<Suspense>` with `<Transition>` in `CompanyDetailPage`**

In `CompanyDetailPage`'s `view!` block, locate the `<Suspense>` that wraps both the uptime chart and probes section (search for `<Suspense` within `CompanyDetailPage` — this wraps `uptime.get()` and `probes.get()`):

```rust
        <Suspense fallback=|| view! { <div class="loading">"Loading…"</div> }>
```

Change to:

```rust
        <Transition fallback=|| view! { <div class="loading">"Loading…"</div> }>
```

Then find the matching closing tag:

```rust
        </Suspense>
```

Change to:

```rust
        </Transition>
```

> **Note:** There is only one `<Suspense>` block in `CompanyDetailPage` that needs changing — the one wrapping both `uptime` and `probes`. The `<Suspense>` blocks in `ArchiveViewerPage` and `AdminPage` are NOT on a polling loop — leave them unchanged.

- [ ] **Step 7: Verify Task 2 compiles**

```bash
cargo check -p hub
```

Expected: no errors.

- [ ] **Step 8: Commit Task 2**

```bash
git add hub/src/app.rs
git commit -m "fix: replace Suspense with Transition in CompanyDetailPage, add interval cleanup"
```

---

### Verification

- [ ] **Step 9: Build and manually verify**

Build the frontend and open the app in a browser:

```bash
cargo leptos watch
```

Navigate to the dashboard. Watch for 15+ seconds. The page content should update in place with no flash/blank. Navigate to a company detail page. Watch for 15+ seconds — same: data updates without blanking.

Navigate away from the company detail page and back. Previously, each visit would add another interval (visible as increasingly rapid updates). After this fix, only one interval fires per page visit.

- [ ] **Step 10: Final check — confirm unchanged pages still work**

Visit the Admin page and Archive viewer page. These still use `<Suspense>` and should load normally (they don't poll, so no flash issue exists there).
