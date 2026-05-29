// OHOS StatusBar API limitations (cannot be fixed at application level):
// - No double-click detection (StatusBar callback has no "doubleClick" type)
// - No hover/enter/leave tracking (only "leftClick"/"rightClick" click_type)
// - No click position coordinates (callback provides only click_type + menuCode)
// - No middle button support (StatusBar has no "middleClick" type)
// - No button press state (callback fires on completed click only)
// Application-internal components have full OHOS gesture/mouse API support,
// but StatusBar tray icon operates through a system-level extension with
// limited callback data.

use crate::{
    dpi::PhysicalPosition, MouseButton, MouseButtonState, Rect, TrayIconEvent, TrayIconId,
};
use crossbeam_channel::select;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::RwLock;
use std::thread;

use super::MENU_METADATA;

static EVENT_THREAD_STARTED: AtomicBool = AtomicBool::new(false);
// RwLock instead of OnceCell: OHOS allows only one tray at a time (singleton),
// but multiple trays may be created/destroyed over the app lifetime.
// Each new tray must update TRAY_ID so events carry the correct ID.
static TRAY_ID: RwLock<Option<TrayIconId>> = RwLock::new(None);

pub fn register_tray_id(id: TrayIconId) {
    let mut guard = TRAY_ID.write().unwrap();
    *guard = Some(id);
}

pub fn get_current_tray_id() -> TrayIconId {
    TRAY_ID
        .read()
        .unwrap()
        .clone()
        .unwrap_or_else(|| TrayIconId::new("main"))
}

pub fn start_event_forward_thread() {
    if EVENT_THREAD_STARTED.swap(true, Ordering::Relaxed) {
        return;
    }

    thread::spawn(move || {
        let icon_receiver = openharmony_ability::statusbar::icon_click_receiver();
        let menu_receiver = openharmony_ability::statusbar::menu_click_receiver();

        loop {
            select! {
                recv(icon_receiver) -> event => {
                    if let Ok(status_bar_event) = event {
                        let tray_event = convert_icon_click(status_bar_event);
                        TrayIconEvent::send(tray_event);
                    }
                },
                recv(menu_receiver) -> event => {
                    if let Ok(status_bar_event) = event {
                        let raw_code = match &status_bar_event {
                            openharmony_ability::statusbar::StatusBarClickEvent::MenuClick { menu_code } => menu_code.clone(),
                            _ => String::new(),
                        };

                        let menu_code = translate_menu_code(&raw_code);

                        let action = {
                            let metadata = MENU_METADATA.lock().unwrap();
                            if let Some(predefined_type) = metadata.predefined_map.get(&menu_code) {
                                MenuAction::Predefined(predefined_type.clone())
                            } else if metadata.check_state.contains_key(&menu_code) {
                                MenuAction::Check(menu_code.clone())
                            } else {
                                MenuAction::Regular
                            }
                        };

                        match action {
                            MenuAction::Predefined(predefined_type) => {
                                log::debug!("[TrayIcon] menu click → predefined: {}", predefined_type);
                                execute_predefined_action(&predefined_type);
                            }
                            MenuAction::Check(code) => {
                                log::debug!("[TrayIcon] menu click → check toggle: {}", code);
                                toggle_check_item(&code);
                                openharmony_ability::send_menu_event(code);
                            }
                            MenuAction::Regular => {
                                log::debug!("[TrayIcon] menu click → regular: {}", menu_code);
                                openharmony_ability::send_menu_event(menu_code);
                            }
                        }
                    }
                },
            }
        }
    });
}

enum MenuAction {
    Predefined(String),
    Check(String),
    Regular,
}

fn execute_predefined_action(predefined_type: &str) {
    log::debug!("[TrayIcon] execute_predefined_action: {}", predefined_type);
    match predefined_type {
        "quit" => {
            let app = super::get_ohos_app();
            app.exit(0);
        }
        "minimize" | "hide" | "maximize" | "close" | "fullscreen" | "about"
        | "copy" | "cut" | "paste" | "selectAll" | "undo" | "redo"
        | "recover" => {
            openharmony_ability::statusbar::execute_predefined_action(predefined_type).ok();
        }
        _ => {
            log::debug!("[TrayIcon] unsupported predefined action: {}", predefined_type);
        }
    }
}

fn toggle_check_item(menu_code: &str) {
    log::debug!("[TrayIcon] toggle_check_item: {}", menu_code);
    let json = {
        let mut metadata = MENU_METADATA.lock().unwrap();
        if let Some(checked) = metadata.check_state.get_mut(menu_code) {
            *checked = !*checked;
        }
        metadata.menu_json.clone()
    };

    if let Some(json) = json {
        rebuild_and_update_menu(&json);
    }
}

fn rebuild_and_update_menu(json: &str) {
    let check_state = {
        let metadata = MENU_METADATA.lock().unwrap();
        metadata.check_state.clone()
    };

    fn patch_items(items: &mut [serde_json::Value], check_state: &std::collections::HashMap<String, bool>) {
        for item in items.iter_mut() {
            if let Some(obj) = item.as_object_mut() {
                if let Some(id) = obj.get("id").and_then(|v| v.as_str()) {
                    if let Some(&checked) = check_state.get(id) {
                        obj.insert("checked".to_string(), serde_json::Value::Bool(checked));
                    }
                }
                if let Some(sub) = obj.get_mut("submenuItems") {
                    if let Some(arr) = sub.as_array_mut() {
                        patch_items(arr, check_state);
                    }
                }
            }
        }
    }

    let mut items: Vec<serde_json::Value> = match serde_json::from_str(json) {
        Ok(v) => v,
        Err(_) => return,
    };
    patch_items(&mut items, &check_state);

    let patched_json = match serde_json::to_string(&items) {
        Ok(j) => j,
        Err(_) => return,
    };

    let menu_items: Vec<super::MenuJsonItem> = match serde_json::from_str(&patched_json) {
        Ok(v) => v,
        Err(_) => return,
    };

    let mut groups = super::split_items_into_groups(menu_items);

    let flat_ids = super::remap_menu_codes_to_indices(&mut groups);
    MENU_METADATA.lock().unwrap().flat_ids = flat_ids;

    if !groups.is_empty() {
        let app = super::get_ohos_app();
        openharmony_ability::statusbar::update_status_bar_menu(app, &groups).ok();
    }
}

fn convert_icon_click(
    _event: openharmony_ability::statusbar::StatusBarClickEvent,
) -> TrayIconEvent {
    TrayIconEvent::Click {
        id: get_current_tray_id(),
        position: PhysicalPosition::new(0.0, 0.0),
        rect: Rect::default(),
        button: MouseButton::Left,
        button_state: MouseButtonState::Up,
    }
}

/// Translate a system-returned numeric menuCode back to our original string ID.
fn translate_menu_code(raw_code: &str) -> String {
    let metadata = MENU_METADATA.lock().unwrap();

    if metadata.flat_ids.is_empty() {
        return raw_code.to_string();
    }

    match raw_code.parse::<usize>() {
        Ok(idx) if idx < metadata.flat_ids.len() => {
            let translated = metadata.flat_ids[idx].clone();
            log::debug!("[TrayIcon] translate: '{}' → '{}' (index {})", raw_code, translated, idx);
            translated
        }
        _ => raw_code.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_icon_click_conversion() {
        let event = openharmony_ability::statusbar::StatusBarClickEvent::IconClick {
            click_type: "leftClick".to_string(),
        };
        let tray_event = convert_icon_click(event);

        match tray_event {
            TrayIconEvent::Click {
                id,
                button,
                button_state,
                position,
                rect,
            } => {
                assert_eq!(id.0, get_current_tray_id().0);
                assert_eq!(button, MouseButton::Left);
                assert_eq!(button_state, MouseButtonState::Up);
                assert_eq!(position.x, 0.0);
                assert_eq!(position.y, 0.0);
                assert_eq!(rect.position.x, 0.0);
                assert_eq!(rect.size.width, 0);
            }
            _ => panic!("unexpected event type"),
        }
    }
}
