use crate::{
    dpi::PhysicalPosition, MouseButton, MouseButtonState, Rect, TrayIconEvent, TrayIconId,
};
use crossbeam_channel::select;
use once_cell::sync::OnceCell;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;

use super::MENU_METADATA;

static EVENT_THREAD_STARTED: AtomicBool = AtomicBool::new(false);
static TRAY_ID: OnceCell<TrayIconId> = OnceCell::new();

pub fn register_tray_id(id: TrayIconId) {
    TRAY_ID.set(id).ok();
}

pub fn get_current_tray_id() -> TrayIconId {
    TRAY_ID
        .get()
        .cloned()
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
                        let menu_code = match &status_bar_event {
                            openharmony_ability::statusbar::StatusBarClickEvent::MenuClick { menu_code } => menu_code.clone(),
                            _ => String::new(),
                        };

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
                                let tray_event = convert_menu_click(status_bar_event);
                                TrayIconEvent::send(tray_event);
                            }
                            MenuAction::Regular => {
                                log::debug!("[TrayIcon] menu click → regular: {}", menu_code);
                                let tray_event = convert_menu_click(status_bar_event);
                                TrayIconEvent::send(tray_event);
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
        "minimize" | "hide" | "maximize" | "close" | "fullscreen" => {
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

    let groups = super::split_items_into_groups(menu_items);

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

fn convert_menu_click(
    event: openharmony_ability::statusbar::StatusBarClickEvent,
) -> TrayIconEvent {
    let menu_code = match event {
        openharmony_ability::statusbar::StatusBarClickEvent::MenuClick { menu_code } => menu_code,
        _ => String::new(),
    };

    // Encode menu_code into the id so it can be retrieved by the application
    let base_id = get_current_tray_id();
    let id = if menu_code.is_empty() {
        base_id
    } else {
        TrayIconId::new(format!("{}:{}", base_id.0, menu_code))
    };

    TrayIconEvent::Click {
        id,
        position: PhysicalPosition::new(0.0, 0.0),
        rect: Rect::default(),
        button: MouseButton::Right,
        button_state: MouseButtonState::Up,
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
                button,
                button_state,
                ..
            } => {
                assert_eq!(button, MouseButton::Left);
                assert_eq!(button_state, MouseButtonState::Up);
            }
            _ => panic!("unexpected event type"),
        }
    }

    #[test]
    fn test_menu_click_conversion() {
        let event = openharmony_ability::statusbar::StatusBarClickEvent::MenuClick {
            menu_code: "item_0".to_string(),
        };
        let tray_event = convert_menu_click(event);

        match tray_event {
            TrayIconEvent::Click {
                button,
                button_state,
                ..
            } => {
                assert_eq!(button, MouseButton::Right);
                assert_eq!(button_state, MouseButtonState::Up);
            }
            _ => panic!("unexpected event type"),
        }
    }
}
