tray-icon lets you create tray icons for desktop applications.

## Platforms supported:

- Windows
- macOS
- Linux (gtk Only)
- FreeBSD (gtk Only)

## Platform-specific notes:

- On Windows and Linux or FreeBSD, an event loop must be running on the thread, on Windows, a win32 event loop and on Linux or FreeBSD, a gtk event loop. It doesn't need to be the main thread but you have to create the tray icon on the same thread as the event loop.
- On macOS, an event loop must be running on the main thread so you also need to create the tray icon on the main thread.
- On OHOS (OpenHarmony), tray (status bar) menu items cannot be disabled: the system `StatusBarMenuItem` API has no `enabled` field, so the enabled flag is ignored on the status bar path and disabled items remain clickable and still fire their commands (a warning is logged once per process when the menu contains disabled items). The OHOS window menu bar and context-menu popups are not affected and honor the enabled flag.
- On OHOS (OpenHarmony), tray (status bar) menus support only one level of submenus: the system `StatusBarSubMenuItem` API has no nested-submenu field, so items nested deeper than one submenu level are dropped and nested titles degrade to clickable leaves (a warning is logged once per process when the menu contains nested submenus). The OHOS window menu bar and context-menu popups are not affected and render any depth.

### OHOS (OpenHarmony): fire-and-forget setters and error observability

On OHOS the tray icon is driven through the statusbar bridge, and every bridge call is dispatched to a single dedicated worker thread (`tray-bridge`, a sequential FIFO queue — see the "Bridge worker thread" comment in `src/platform_impl/ohos/mod.rs`). Two consequences for the public API:

- `TrayIcon::set_icon`, `TrayIcon::set_visible` and `TrayIcon::set_tooltip` return immediately. The returned `Ok` reflects only local synchronous validation (icon encoding, the 128-char tooltip limit, item rebuild); the outcome of the actual statusbar bridge call is never propagated back to the caller — the call is queued on the worker thread (fire-and-forget) and the setter returns before the bridge has run.
- This is deliberate: a synchronous per-call receipt would have to block the calling thread until the worker's bridge response arrives. The main (ArkTS/N-API) thread owns the TSFN queue, and blocking it with `block_on` prevents exactly the TSFN callbacks that deliver bridge responses, deadlocking against the `THREAD_BLOCK_3S` watchdog (see the same worker-thread comment in `src/platform_impl/ohos/mod.rs`). Synchronous receipts are feasible off the main thread — the statusbar plugin's `call::<_, StatusBarAcknowledgement>` pattern is the existing precedent — but the tray setters can be called from any thread and cannot assume that.

Bridge failures remain observable through two channels that exist today:

- `tray_icon::last_bridge_error()` (available when `target_env = "ohos"`) returns the last recorded bridge failure as `Option<crate::Error>`. The slot is last-wins: a newer failure overwrites an older one, and reading it does not clear it. It is a diagnostics channel, not a per-call receipt — it tells you what failed most recently, not which specific call failed.
- the worker logs every bridge failure through the `log` crate (`log::warn!("[TrayIcon] <op> error: ...")`, or `log::error!` when the statusbar client was never initialized), so each failure appears in any subscribed logger as it happens.

A per-call opt-in receipt API (e.g. `set_icon_with_receipt(timeout) -> Result`, backed by a oneshot/mpsc channel with `recv_timeout`, never callable on the main thread) is planned as a follow-up issue.

### Cargo Features

- `common-controls-v6`: Use `TaskDialogIndirect` API from `ComCtl32.dll` v6 on Windows for showing the predefined `About` menu item dialog.
- `libxdo`: Enables linking to `libxdo` which is used for the predfined `Copy`, `Cut`, `Paste` and `SelectAll` menu item, see https://github.com/tauri-apps/muda#cargo-features
- `serde`: Enables de/serializing derives.

## Dependencies (Linux Only)

On Linux, `gtk`, `libxdo` is used to make the predfined `Copy`, `Cut`, `Paste` and `SelectAll` menu items work and `libappindicator` or `libayatnat-appindicator` are used to create the tray icon, so make sure to install them on your system.

#### Arch Linux / Manjaro:

```sh
pacman -S gtk3 xdotool libappindicator-gtk3 #or libayatana-appindicator
```

#### Debian / Ubuntu:

```sh
sudo apt install libgtk-3-dev libxdo-dev libappindicator3-dev #or libayatana-appindicator3-dev
```

## Dependencies in FreeBSD

Install this dependencies in order to compile `tray-icon`. Instructions using `pkg`:

```sh
pkg install -y rust glib pkgconf gtk3
```

## Examples

#### Create a tray icon without a menu.

```rs
use tray_icon::TrayIconBuilder;

let tray_icon = TrayIconBuilder::new()
    .with_tooltip("system-tray - tray icon library!")
    .with_icon(icon)
    .build()
    .unwrap();
```

#### Create a tray icon with a menu.

```rs
use tray_icon::{TrayIconBuilder, menu::Menu};

let tray_menu = Menu::new();
let tray_icon = TrayIconBuilder::new()
    .with_menu(Box::new(tray_menu))
    .with_tooltip("system-tray - tray icon library!")
    .with_icon(icon)
    .build()
    .unwrap();
```

## Processing tray events

You can use `TrayIconEvent::receiver` to get a reference to the `TrayIconEventReceiver`
which you can use to listen to events when a click happens on the tray icon

```rs
use tray_icon::TrayIconEvent;

if let Ok(event) = TrayIconEvent::receiver().try_recv() {
    println!("{:?}", event);
}
```

You can also listen for the menu events using `MenuEvent::receiver` to get events for the tray context menu.

```rs
use tray_icon::{TrayIconEvent, menu::{MenuEvent}};

if let Ok(event) = TrayIconEvent::receiver().try_recv() {
    println!("tray event: {:?}", event);
}

if let Ok(event) = MenuEvent::receiver().try_recv() {
    println!("menu event: {:?}", event);
}
```

### Note for [winit] or [tao] users:

You should use [`TrayIconEvent::set_event_handler`] and forward
the tray icon events to the event loop by using [`EventLoopProxy`]
so that the event loop is awakened on each tray icon event.
Same can be done for menu events using [`MenuEvent::set_event_handler`].

```rust
enum UserEvent {
  TrayIconEvent(tray_icon::TrayIconEvent)
  MenuEvent(tray_icon::menu::MenuEvent)
}

let event_loop = EventLoop::<UserEvent>::with_user_event().build().unwrap();

let proxy = event_loop.create_proxy();
tray_icon::TrayIconEvent::set_event_handler(Some(move |event| {
    proxy.send_event(UserEvent::TrayIconEvent(event));
}));

let proxy = event_loop.create_proxy();
tray_icon::menu::MenuEvent::set_event_handler(Some(move |event| {
    proxy.send_event(UserEvent::MenuEvent(event));
}));
```

[`EventLoopProxy`]: https://docs.rs/winit/latest/winit/event_loop/struct.EventLoopProxy.html
[winit]: https://docs.rs/winit
[tao]: https://docs.rs/tao

## License

Apache-2.0/MIT
