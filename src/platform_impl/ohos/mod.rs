mod event;
mod icon;

pub(crate) use icon::PlatformIcon;

use crate::{TrayIconAttributes, TrayIconId};
use once_cell::sync::OnceCell;
use std::cell::RefCell;
use std::collections::HashMap;
use std::io;
use std::sync::Mutex;

static OHOS_APP: OnceCell<openharmony_ability::OpenHarmonyApp> = OnceCell::new();

pub(crate) static MENU_METADATA: once_cell::sync::Lazy<Mutex<MenuMetadata>> =
    once_cell::sync::Lazy::new(|| Mutex::new(MenuMetadata::default()));

#[derive(Default)]
pub(crate) struct MenuMetadata {
    pub predefined_map: HashMap<String, String>,
    pub check_state: HashMap<String, bool>,
    pub menu_json: Option<String>,
    pub flat_ids: Vec<String>,
}

pub fn set_ohos_app(app: openharmony_ability::OpenHarmonyApp) {
    OHOS_APP.set(app).expect("OHOS_APP already set");
}

pub(crate) fn get_ohos_app() -> &'static openharmony_ability::OpenHarmonyApp {
    OHOS_APP.get().expect("OHOS_APP not initialized")
}

pub struct TrayIcon {
    attrs: RefCell<TrayIconAttributes>,
    is_visible: RefCell<bool>,
}

impl TrayIcon {
    pub fn new(id: TrayIconId, attrs: TrayIconAttributes) -> crate::Result<Self> {
        let app = get_ohos_app();

        // Extract metadata before building the item (menus may be consumed)
        let (predefined_map, check_state, menu_json) =
            extract_menu_metadata(&attrs.menu);

        {
            let mut metadata = MENU_METADATA.lock().unwrap();
            metadata.predefined_map = predefined_map;
            metadata.check_state = check_state;
            metadata.menu_json = menu_json;
        }

        let mut item = build_item_from_attrs(&attrs)?;

        if let Some(ref mut groups) = item.status_bar_group_menu {
            let flat_ids = remap_menu_codes_to_indices(groups);
            MENU_METADATA.lock().unwrap().flat_ids = flat_ids;
        }

        openharmony_ability::statusbar::add_to_status_bar(app, &item)
            .map_err(|e| crate::Error::OhosError(e.to_string()))?;

        event::register_tray_id(id);
        event::start_event_forward_thread();

        Ok(Self {
            attrs: RefCell::new(attrs),
            is_visible: RefCell::new(true),
        })
    }

    pub fn set_icon(&mut self, icon: Option<crate::Icon>) -> crate::Result<()> {
        let app = get_ohos_app();
        let is_template = self.attrs.borrow().icon_is_template;
        if let Some(i) = &icon {
            let status_bar_icon = icon::icon_to_status_bar_icon(&i.inner, is_template)?;
            openharmony_ability::statusbar::update_status_bar_icon(app, &status_bar_icon)
                .map_err(|e| crate::Error::OhosError(e.to_string()))?;
        } else {
            // Clear icon by sending empty icon data
            let empty_icon = openharmony_ability::statusbar::StatusBarIcon::default();
            openharmony_ability::statusbar::update_status_bar_icon(app, &empty_icon)
                .map_err(|e| crate::Error::OhosError(e.to_string()))?;
        }
        self.attrs.borrow_mut().icon = icon;
        Ok(())
    }

    pub fn set_menu(&mut self, menu: Option<Box<dyn crate::menu::ContextMenu>>) {
        let app = get_ohos_app();
        let (menus, predefined_map, check_state, menu_json) =
            menu_to_status_bar_items_with_metadata(&menu);

        {
            let mut metadata = MENU_METADATA.lock().unwrap();
            metadata.predefined_map = predefined_map;
            metadata.check_state = check_state;
            metadata.menu_json = menu_json;
        }

        if let Some(mut m) = menus {
            let flat_ids = remap_menu_codes_to_indices(&mut m);
            MENU_METADATA.lock().unwrap().flat_ids = flat_ids;
            openharmony_ability::statusbar::update_status_bar_menu(app, &m)
                .map_err(|e| crate::Error::OhosError(e.to_string()))
                .ok();
        } else if menu.is_none() {
            openharmony_ability::statusbar::update_status_bar_menu(app, &vec![])
                .map_err(|e| crate::Error::OhosError(e.to_string()))
                .ok();
        }
        self.attrs.borrow_mut().menu = menu;
    }

    pub fn set_tooltip<S: AsRef<str>>(&mut self, tooltip: Option<S>) -> crate::Result<()> {
        let app = get_ohos_app();
        let tips = tooltip.and_then(|s| {
            let s = s.as_ref().to_string();
            if s.is_empty() { None } else { Some(s) }
        });
        if let Some(ref t) = tips {
            if t.len() <= 128 {
                openharmony_ability::statusbar::update_hover_tips(app, t)
                    .map_err(|e| crate::Error::OhosError(e.to_string()))?;
            }
        }
        self.attrs.borrow_mut().tooltip = tips;
        Ok(())
    }

    pub fn set_title<S: AsRef<str>>(&mut self, title: Option<S>) {
        if let Some(t) = title {
            self.attrs.borrow_mut().title = Some(t.as_ref().to_string());
        } else {
            self.attrs.borrow_mut().title = None;
        }
        if *self.is_visible.borrow() {
            let app = get_ohos_app();
            openharmony_ability::statusbar::remove_from_status_bar(app)
                .map_err(|e| log::warn!("[TrayIcon] remove error in set_title: {}", e))
                .ok();
            let item = build_item_from_attrs(&self.attrs.borrow()).ok();
            if let Some(item) = item {
                openharmony_ability::statusbar::add_to_status_bar(app, &item)
                    .map_err(|e| log::warn!("[TrayIcon] add error in set_title: {}", e))
                    .ok();
            }
        }
    }

    pub fn set_visible(&mut self, visible: bool) -> crate::Result<()> {
        let app = get_ohos_app();

        if visible && !*self.is_visible.borrow() {
            let item = build_item_from_attrs(&self.attrs.borrow())?;
            openharmony_ability::statusbar::add_to_status_bar(app, &item)
                .map_err(|e| crate::Error::OhosError(e.to_string()))?;
            *self.is_visible.borrow_mut() = true;
        } else if !visible && *self.is_visible.borrow() {
            openharmony_ability::statusbar::remove_from_status_bar(app)
                .map_err(|e| crate::Error::OhosError(e.to_string()))
                .ok();
            *self.is_visible.borrow_mut() = false;
        }
        Ok(())
    }

    pub fn set_quick_operation(&mut self, config: Option<crate::QuickOperationConfig>) {
        self.attrs.borrow_mut().quick_operation = config;
        if *self.is_visible.borrow() {
            let app = get_ohos_app();
            openharmony_ability::statusbar::remove_from_status_bar(app)
                .map_err(|e| log::warn!("[TrayIcon] remove error in set_quick_operation: {}", e))
                .ok();
            let item = build_item_from_attrs(&self.attrs.borrow()).ok();
            if let Some(item) = item {
                openharmony_ability::statusbar::add_to_status_bar(app, &item)
                    .map_err(|e| log::warn!("[TrayIcon] add error in set_quick_operation: {}", e))
                    .ok();
            }
        }
    }

    pub fn set_temp_dir_path<P: AsRef<std::path::Path>>(&mut self, _path: Option<P>) {}

    pub fn set_icon_as_template(&mut self, is_template: bool) -> crate::Result<()> {
        // No-op if value unchanged — avoids unnecessary remove+re-add
        if self.attrs.borrow().icon_is_template == is_template {
            return Ok(());
        }
        self.attrs.borrow_mut().icon_is_template = is_template;
        if *self.is_visible.borrow() {
            let app = get_ohos_app();
            openharmony_ability::statusbar::remove_from_status_bar(app)
                .map_err(|e| log::warn!("[TrayIcon] remove error in set_icon_as_template: {}", e))
                .ok();
            let item = build_item_from_attrs(&self.attrs.borrow())?;
            openharmony_ability::statusbar::add_to_status_bar(app, &item)
                .map_err(|e| crate::Error::OhosError(e.to_string()))?;
        }
        Ok(())
    }

    pub fn set_icon_with_as_template(
        &mut self,
        icon: Option<crate::Icon>,
        is_template: bool,
    ) -> crate::Result<()> {
        self.attrs.borrow_mut().icon_is_template = is_template;
        self.set_icon(icon)
    }

    // OHOS: rect() always returns None.
    // StatusBar API does not provide tray icon position or dimensions.
    // AvoidArea.topRect returns the entire status bar area (e.g. {0,0,1440,48}),
    // not the tray icon itself — using it as an approximation would mislead callers
    // who rely on rect for popup positioning or size calculations.
    // This is consistent with Linux, which also returns None.
    pub fn rect(&self) -> Option<crate::Rect> {
        None
    }
}

impl Drop for TrayIcon {
    fn drop(&mut self) {
        if *self.is_visible.borrow() {
            let app = get_ohos_app();
            openharmony_ability::statusbar::remove_from_status_bar(app)
                .map_err(|e| log::warn!("[TrayIcon] remove_from_status_bar error: {}", e))
                .ok();
            openharmony_ability::statusbar::unregister_icon_click_handler()
                .map_err(|e| log::warn!("[TrayIcon] unregister_icon_click error: {}", e))
                .ok();
            openharmony_ability::statusbar::unregister_menu_click_handler()
                .map_err(|e| log::warn!("[TrayIcon] unregister_menu_click error: {}", e))
                .ok();
        }
    }
}

fn menu_to_status_bar_items(
    menu: &Option<Box<dyn crate::menu::ContextMenu>>,
) -> Option<Vec<Vec<openharmony_ability::statusbar::StatusBarMenuItem>>> {
    menu.as_ref().and_then(|m| {
        let json = m.ohos_context_menu();
        let items: Vec<MenuJsonItem> = serde_json::from_str(&json).unwrap_or_default();
        if items.is_empty() {
            None
        } else {
            let groups = split_items_into_groups(items);
            if groups.is_empty() {
                None
            } else {
                Some(groups)
            }
        }
    })
}

/// Extract menu metadata (predefined_map, check_state, menu_json) without
/// converting to StatusBarMenuItem. Used by `new()` which delegates the
/// conversion to `build_item_from_attrs()`.
fn extract_menu_metadata(
    menu: &Option<Box<dyn crate::menu::ContextMenu>>,
) -> (
    HashMap<String, String>,
    HashMap<String, bool>,
    Option<String>,
) {
    let Some(m) = menu.as_ref() else {
        return (HashMap::new(), HashMap::new(), None);
    };
    let json = m.ohos_context_menu();
    let items: Vec<MenuJsonItem> = serde_json::from_str(&json).unwrap_or_default();
    if items.is_empty() {
        return (HashMap::new(), HashMap::new(), None);
    }

    let mut predefined_map = HashMap::new();
    let mut check_state = HashMap::new();
    collect_metadata_from_items(&items, &mut predefined_map, &mut check_state);

    (predefined_map, check_state, Some(json))
}

fn menu_to_status_bar_items_with_metadata(
    menu: &Option<Box<dyn crate::menu::ContextMenu>>,
) -> (
    Option<Vec<Vec<openharmony_ability::statusbar::StatusBarMenuItem>>>,
    HashMap<String, String>,
    HashMap<String, bool>,
    Option<String>,
) {
    let empty = (None, HashMap::new(), HashMap::new(), None);
    let Some(m) = menu.as_ref() else {
        return empty;
    };
    let json = m.ohos_context_menu();
    let items: Vec<MenuJsonItem> = serde_json::from_str(&json).unwrap_or_default();
    if items.is_empty() {
        return empty;
    }

    let mut predefined_map = HashMap::new();
    let mut check_state = HashMap::new();

    collect_metadata_from_items(&items, &mut predefined_map, &mut check_state);

    let groups = split_items_into_groups(items);

    let result = if groups.is_empty() {
        None
    } else {
        Some(groups)
    };

    (result, predefined_map, check_state, Some(json))
}

fn collect_metadata_from_items(
    items: &[MenuJsonItem],
    predefined_map: &mut HashMap<String, String>,
    check_state: &mut HashMap<String, bool>,
) {
    for item in items {
        log::debug!("[TrayIcon] collect_metadata: id={}, type={}, predefined_type={:?}", item.id, item.item_type, item.predefined_type);
        if item.item_type == "predefined" {
            if let Some(ref pt) = item.predefined_type {
                if pt != "separator" {
                    predefined_map.insert(item.id.clone(), pt.clone());
                    log::debug!("[TrayIcon]   → predefined_map: {} → {}", item.id, pt);
                }
            }
        } else if item.item_type == "check" {
            check_state.insert(item.id.clone(), item.checked.unwrap_or(false));
            log::debug!("[TrayIcon]   → check_state: {} → {}", item.id, item.checked.unwrap_or(false));
        }
        if let Some(ref children) = item.submenu_items {
            collect_metadata_from_items(children, predefined_map, check_state);
        }
    }
}

pub(crate) fn split_items_into_groups(
    items: Vec<MenuJsonItem>,
) -> Vec<Vec<openharmony_ability::statusbar::StatusBarMenuItem>> {
    let mut groups: Vec<Vec<openharmony_ability::statusbar::StatusBarMenuItem>> = Vec::new();
    let mut current_group: Vec<openharmony_ability::statusbar::StatusBarMenuItem> = Vec::new();

    for item in items {
        if is_separator(&item) {
            if !current_group.is_empty() {
                groups.push(current_group);
                current_group = Vec::new();
            }
        } else {
            current_group.push(menu_json_item_to_status_bar_item(item));
        }
    }
    if !current_group.is_empty() {
        groups.push(current_group);
    }

    groups
}

/// Replace all menuCode values with sequential numeric strings ("0", "1", ...).
/// Returns the flat_ids mapping: index → our original string ID.
pub(crate) fn remap_menu_codes_to_indices(
    groups: &mut [Vec<openharmony_ability::statusbar::StatusBarMenuItem>],
) -> Vec<String> {
    let mut flat_ids = Vec::new();
    let mut idx: usize = 0;
    for group in groups.iter_mut() {
        for item in group.iter_mut() {
            if let Some(ref code) = item.menu_code {
                flat_ids.push(code.clone());
                let num = idx.to_string();
                item.menu_code = Some(num.clone());
                if let Some(ref mut action) = item.menu_action {
                    action.menu_code = Some(num);
                }
                idx += 1;
            }
            if let Some(ref mut sub_menu) = item.sub_menu {
                for sub in sub_menu.iter_mut() {
                    if let Some(ref code) = sub.menu_code {
                        flat_ids.push(code.clone());
                        let num = idx.to_string();
                        sub.menu_code = Some(num.clone());
                        sub.menu_action.menu_code = Some(num);
                        idx += 1;
                    }
                }
            }
        }
    }
    log::debug!("[TrayIcon] remap_menu_codes: {} items → {:?}", idx, flat_ids);
    flat_ids
}

fn strip_mnemonics(text: &str) -> String {
    text.replace('&', "")
}

fn decode_png_to_rgba(png_bytes: &[u8]) -> Result<(Vec<u8>, u32, u32), String> {
    let decoder = png::Decoder::new(std::io::Cursor::new(png_bytes));
    let mut reader = decoder.read_info().map_err(|e| format!("PNG read_info failed: {}", e))?;
    let buf_size = reader.output_buffer_size().ok_or("PNG output_buffer_size failed")?;
    let mut buf = vec![0u8; buf_size];
    let info = reader.next_frame(&mut buf).map_err(|e| format!("PNG next_frame failed: {}", e))?;
    let width = info.width;
    let height = info.height;

    let rgba = match info.color_type {
        png::ColorType::Rgba => buf[..info.buffer_size()].to_vec(),
        png::ColorType::Rgb => {
            let rgb = &buf[..info.buffer_size()];
            let mut rgba = Vec::with_capacity(width as usize * height as usize * 4);
            for chunk in rgb.chunks(3) {
                rgba.extend_from_slice(chunk);
                rgba.push(255);
            }
            rgba
        }
        _ => return Err(format!("Unsupported PNG color type: {:?}", info.color_type)),
    };

    Ok((rgba, width, height))
}

#[derive(serde::Deserialize)]
#[allow(dead_code)]
pub(crate) struct MenuJsonItem {
    id: String,
    #[serde(default)]
    text: Option<String>,
    #[serde(rename = "type", default)]
    item_type: String,
    #[serde(default)]
    enabled: Option<bool>,
    #[serde(default)]
    accelerator: Option<String>,
    #[serde(rename = "predefinedType")]
    predefined_type: Option<String>,
    #[serde(rename = "submenuItems")]
    submenu_items: Option<Vec<MenuJsonItem>>,
    #[serde(default)]
    checked: Option<bool>,
    #[serde(default)]
    icon: Option<String>,
    #[serde(rename = "aboutMetadata", default)]
    about_metadata: Option<AboutMetadataJson>,
}

#[derive(serde::Deserialize)]
#[allow(dead_code)]
pub(crate) struct AboutMetadataJson {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    version: Option<String>,
    #[serde(rename = "shortVersion", default)]
    short_version: Option<String>,
    #[serde(default)]
    authors: Option<Vec<String>>,
    #[serde(default)]
    comments: Option<String>,
    #[serde(default)]
    copyright: Option<String>,
    #[serde(default)]
    license: Option<String>,
    #[serde(default)]
    website: Option<String>,
}

pub(crate) fn is_separator(item: &MenuJsonItem) -> bool {
    item.item_type == "separator"
        || (item.item_type == "predefined"
            && item.predefined_type.as_deref() == Some("separator"))
}

pub(crate) fn menu_json_item_to_status_bar_item(
    item: MenuJsonItem,
) -> openharmony_ability::statusbar::StatusBarMenuItem {
    if item.item_type == "submenu" {
        let sub_items: Vec<openharmony_ability::statusbar::StatusBarSubMenuItem> = item
            .submenu_items
            .unwrap_or_default()
            .into_iter()
            .filter(|child| !is_separator(child))
            .map(|child| {
                let text = strip_mnemonics(&child.text.unwrap_or_default());
                let options = build_item_options(&child.item_type, child.checked, child.icon.as_deref());
                openharmony_ability::statusbar::StatusBarSubMenuItem {
                    sub_title: text,
                    menu_code: Some(child.id.clone()),
                    menu_action: openharmony_ability::statusbar::StatusBarMenuAction {
                        ability_name: String::new(),
                        module_name: None,
                        menu_code: Some(child.id),
                        notify_only: Some(true),
                    },
                    options,
                }
            })
            .collect();

        openharmony_ability::statusbar::StatusBarMenuItem {
            title: strip_mnemonics(&item.text.unwrap_or_default()),
            menu_code: None,
            sub_menu: Some(sub_items),
            menu_action: None,
            options: None,
        }
    } else {
        let options = build_item_options(&item.item_type, item.checked, item.icon.as_deref());
        openharmony_ability::statusbar::StatusBarMenuItem {
            title: strip_mnemonics(&item.text.unwrap_or_default()),
            menu_code: Some(item.id.clone()),
            sub_menu: None,
            menu_action: Some(openharmony_ability::statusbar::StatusBarMenuAction {
                ability_name: String::new(),
                module_name: None,
                menu_code: Some(item.id),
                notify_only: Some(true),
            }),
            options,
        }
    }
}

fn build_item_options(
    item_type: &str,
    checked: Option<bool>,
    icon_b64: Option<&str>,
) -> Option<openharmony_ability::statusbar::StatusBarMenuItemOptions> {
    log::debug!("[TrayIcon] build_item_options: type={}, checked={:?}, has_icon={}", item_type, checked, icon_b64.is_some());
    let selected = if item_type == "check" { checked } else { None };

    let (icon_rgba, icon_width, icon_height) = if item_type == "icon" {
        decode_icon_from_base64(icon_b64)
    } else {
        (None, None, None)
    };

    if selected.is_some() || icon_rgba.is_some() {
        Some(openharmony_ability::statusbar::StatusBarMenuItemOptions {
            icon: None,
            selected,
            selected_icon: None,
            icon_rgba,
            icon_width,
            icon_height,
        })
    } else {
        None
    }
}

fn decode_icon_from_base64(icon_b64: Option<&str>) -> (Option<Vec<u8>>, Option<u32>, Option<u32>) {
    let Some(b64) = icon_b64 else {
        log::debug!("[TrayIcon] decode_icon_from_base64: no base64 data");
        return (None, None, None);
    };
    log::debug!("[TrayIcon] decode_icon_from_base64: b64 len={}", b64.len());
    use base64::Engine;
    let Ok(png_bytes) = base64::engine::general_purpose::STANDARD.decode(b64) else {
        log::debug!("[TrayIcon] decode_icon_from_base64: base64 decode failed");
        return (None, None, None);
    };
    log::debug!("[TrayIcon] decode_icon_from_base64: png_bytes len={}", png_bytes.len());
    match decode_png_to_rgba(&png_bytes) {
        Ok((rgba, width, height)) => {
            log::debug!("[TrayIcon] decode_icon_from_base64: decoded {}x{}, rgba len={}", width, height, rgba.len());
            (Some(rgba), Some(width), Some(height))
        }
        Err(e) => {
            log::debug!("[TrayIcon] decode_icon_from_base64: PNG decode failed: {}", e);
            (None, None, None)
        }
    }
}

fn build_item_from_attrs(
    attrs: &TrayIconAttributes,
) -> crate::Result<openharmony_ability::statusbar::StatusBarItem> {
    let icon = attrs.icon.as_ref().ok_or_else(|| {
        crate::Error::OsError(io::Error::new(
            io::ErrorKind::InvalidData,
            "No icon provided",
        ))
    })?;

    let status_bar_icon = icon::icon_to_status_bar_icon(&icon.inner, attrs.icon_is_template)?;

    let quick_operation = if let Some(ref config) = attrs.quick_operation {
        openharmony_ability::statusbar::QuickOperation {
            ability_name: config.ability_name.clone(),
            title: if config.title.is_empty() {
                attrs.title.clone().unwrap_or_else(|| "Tauri App".to_string())
            } else {
                config.title.clone()
            },
            height: config.height,
            module_name: config.module_name.clone(),
            loading_status: config.loading_status,
        }
    } else {
        openharmony_ability::statusbar::QuickOperation {
            ability_name: String::new(),
            title: attrs
                .title
                .clone()
                .unwrap_or_else(|| "Tauri App".to_string()),
            height: 200,
            module_name: Some("entry".to_string()),
            loading_status: None,
        }
    };

    let menus = menu_to_status_bar_items(&attrs.menu);

    Ok(openharmony_ability::statusbar::StatusBarItem {
        icons: status_bar_icon,
        quick_operation,
        status_bar_group_menu: menus,
        hover_tips: attrs.tooltip.clone().filter(|s| !s.is_empty()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_item(id: &str, text: &str) -> MenuJsonItem {
        MenuJsonItem {
            id: id.to_string(),
            text: Some(text.to_string()),
            enabled: Some(true),
            accelerator: None,
            item_type: "item".to_string(),
            predefined_type: None,
            submenu_items: None,
            checked: None,
            icon: None,
            about_metadata: None,
        }
    }

    fn make_separator(id: &str) -> MenuJsonItem {
        MenuJsonItem {
            id: id.to_string(),
            text: None,
            enabled: Some(false),
            accelerator: None,
            item_type: "predefined".to_string(),
            predefined_type: Some("separator".to_string()),
            submenu_items: None,
            checked: None,
            icon: None,
            about_metadata: None,
        }
    }

    fn make_submenu(id: &str, text: &str, children: Vec<MenuJsonItem>) -> MenuJsonItem {
        MenuJsonItem {
            id: id.to_string(),
            text: Some(text.to_string()),
            enabled: Some(true),
            accelerator: None,
            item_type: "submenu".to_string(),
            predefined_type: None,
            submenu_items: Some(children),
            checked: None,
            icon: None,
            about_metadata: None,
        }
    }

    #[test]
    fn test_regular_item_becomes_top_level_menu_item() {
        let item = make_item("item_1", "Open");
        let result = menu_json_item_to_status_bar_item(item);

        assert_eq!(result.title, "Open");
        assert!(result.sub_menu.is_none());
        assert!(result.menu_action.is_some());
        let action = result.menu_action.unwrap();
        assert_eq!(action.menu_code, Some("item_1".to_string()));
        assert!(action.notify_only.unwrap());
    }

    #[test]
    fn test_submenu_becomes_item_with_sub_menu() {
        let item = make_submenu("submenu_1", "File", vec![
            make_item("item_new", "New"),
            make_item("item_open", "Open"),
        ]);
        let result = menu_json_item_to_status_bar_item(item);

        assert_eq!(result.title, "File");
        assert!(result.menu_action.is_none());
        assert!(result.sub_menu.is_some());
        let sub_items = result.sub_menu.unwrap();
        assert_eq!(sub_items.len(), 2);
        assert_eq!(sub_items[0].sub_title, "New");
        assert_eq!(sub_items[0].menu_code, Some("item_new".to_string()));
        assert_eq!(sub_items[1].sub_title, "Open");
        assert_eq!(sub_items[1].menu_code, Some("item_open".to_string()));
    }

    #[test]
    fn test_separators_filtered_out() {
        let items = vec![
            make_item("item_1", "Copy"),
            make_separator("sep_1"),
            make_item("item_2", "Paste"),
        ];
        let menu_items: Vec<_> = items
            .into_iter()
            .filter(|item| !is_separator(item))
            .map(|item| menu_json_item_to_status_bar_item(item))
            .collect();

        assert_eq!(menu_items.len(), 2);
        assert_eq!(menu_items[0].title, "Copy");
        assert_eq!(menu_items[1].title, "Paste");
    }

    #[test]
    fn test_menu_json_deserialization_from_muda_format() {
        let json = r#"[
            {"id":"1","type":"item","text":"Open","enabled":true},
            {"id":"2","type":"predefined","text":"","enabled":true,"predefinedType":"separator"},
            {"id":"3","type":"submenu","text":"File","enabled":true,"submenuItems":[
                {"id":"3a","type":"item","text":"New","enabled":true}
            ]}
        ]"#;
        let items: Vec<MenuJsonItem> = serde_json::from_str(json).unwrap();
        assert_eq!(items.len(), 3);
        assert_eq!(items[0].item_type, "item");
        assert_eq!(items[0].text, Some("Open".to_string()));
        assert_eq!(items[1].item_type, "predefined");
        assert_eq!(items[1].predefined_type, Some("separator".to_string()));
        assert_eq!(items[2].item_type, "submenu");
        assert_eq!(items[2].submenu_items.as_ref().unwrap().len(), 1);
    }

    #[test]
    fn test_split_items_into_groups_at_separator() {
        let items = vec![
            make_item("item_1", "Copy"),
            make_separator("sep_1"),
            make_item("item_2", "Paste"),
        ];
        let groups = split_items_into_groups(items);
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].len(), 1);
        assert_eq!(groups[0][0].title, "Copy");
        assert_eq!(groups[1].len(), 1);
        assert_eq!(groups[1][0].title, "Paste");
    }

    #[test]
    fn build_item_with_quick_operation() {
        use crate::QuickOperationConfig;
        let mut attrs = TrayIconAttributes::default();
        attrs.quick_operation = Some(QuickOperationConfig {
            title: "My Panel".into(),
            height: 300,
            ability_name: "MyAbility".into(),
            module_name: Some("entry".into()),
            loading_status: Some(true),
        });

        // Verify the QuickOperation struct that build_item_from_attrs would create
        let config = attrs.quick_operation.as_ref().unwrap();
        let qo = openharmony_ability::statusbar::QuickOperation {
            ability_name: config.ability_name.clone(),
            title: if config.title.is_empty() {
                attrs.title.clone().unwrap_or_else(|| "Tauri App".to_string())
            } else {
                config.title.clone()
            },
            height: config.height,
            module_name: config.module_name.clone(),
            loading_status: config.loading_status,
        };

        assert_eq!(qo.ability_name, "MyAbility");
        assert_eq!(qo.title, "My Panel");
        assert_eq!(qo.height, 300);
        assert_eq!(qo.module_name, Some("entry".into()));
        assert_eq!(qo.loading_status, Some(true));
    }

    #[test]
    fn build_item_without_quick_operation() {
        let mut attrs = TrayIconAttributes::default();
        attrs.title = Some("My App".into());
        // quick_operation is None by default

        // Verify the default QuickOperation struct that build_item_from_attrs would create
        let qo = openharmony_ability::statusbar::QuickOperation {
            ability_name: String::new(),
            title: attrs
                .title
                .clone()
                .unwrap_or_else(|| "Tauri App".to_string()),
            height: 200,
            module_name: Some("entry".to_string()),
            loading_status: None,
        };

        assert!(qo.ability_name.is_empty());
        assert_eq!(qo.title, "My App");
        assert_eq!(qo.height, 200);
        assert_eq!(qo.module_name, Some("entry".to_string()));
        assert!(qo.loading_status.is_none());
    }

    #[test]
    fn quick_operation_empty_title_falls_back_to_attrs_title() {
        use crate::QuickOperationConfig;
        let mut attrs = TrayIconAttributes::default();
        attrs.title = Some("App Title".into());
        attrs.quick_operation = Some(QuickOperationConfig {
            title: String::new(), // empty → should fall back to attrs.title
            height: 250,
            ability_name: "TestAbility".into(),
            module_name: None,
            loading_status: None,
        });

        let config = attrs.quick_operation.as_ref().unwrap();
        let title = if config.title.is_empty() {
            attrs.title.clone().unwrap_or_else(|| "Tauri App".to_string())
        } else {
            config.title.clone()
        };

        assert_eq!(title, "App Title");
    }

    #[test]
    fn quick_operation_no_attrs_title_falls_back_to_default() {
        use crate::QuickOperationConfig;
        let mut attrs = TrayIconAttributes::default();
        // attrs.title is None
        attrs.quick_operation = Some(QuickOperationConfig {
            title: String::new(), // empty
            height: 250,
            ability_name: "TestAbility".into(),
            module_name: None,
            loading_status: None,
        });

        let config = attrs.quick_operation.as_ref().unwrap();
        let title = if config.title.is_empty() {
            attrs.title.clone().unwrap_or_else(|| "Tauri App".to_string())
        } else {
            config.title.clone()
        };

        assert_eq!(title, "Tauri App");
    }

    #[test]
    fn test_icon_is_template_default_false() {
        let attrs = TrayIconAttributes::default();
        assert_eq!(attrs.icon_is_template, false);
    }

    #[test]
    fn test_build_item_from_attrs_template_mode() {
        use crate::icon::Icon;

        // Create a simple 2x2 red icon
        let rgba = vec![255u8, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 128, 128, 128, 128];
        let icon = Icon::from_rgba(rgba, 2, 2).unwrap();

        let mut attrs = TrayIconAttributes::default();
        attrs.icon = Some(icon);
        attrs.icon_is_template = true;

        let item = build_item_from_attrs(&attrs).unwrap();
        let white = item.icons.white.borrow().clone().unwrap();
        let black = item.icons.black.borrow().clone().unwrap();

        // Template mode: white and black should differ
        assert_ne!(white, black);
        // White version: RGB=255
        assert_eq!(white[0], 255);
        assert_eq!(white[1], 255);
        assert_eq!(white[2], 255);
        // Black version: RGB=0
        assert_eq!(black[0], 0);
        assert_eq!(black[1], 0);
        assert_eq!(black[2], 0);
    }

    #[test]
    fn test_build_item_from_attrs_not_template() {
        use crate::icon::Icon;

        let rgba = vec![255u8, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 128, 128, 128, 128];
        let icon = Icon::from_rgba(rgba.clone(), 2, 2).unwrap();

        let mut attrs = TrayIconAttributes::default();
        attrs.icon = Some(icon);
        attrs.icon_is_template = false;

        let item = build_item_from_attrs(&attrs).unwrap();
        let white = item.icons.white.borrow().clone().unwrap();
        let black = item.icons.black.borrow().clone().unwrap();

        // Not template: white and black should be the same original image
        assert_eq!(white, black);
        assert_eq!(white, rgba);
    }
}
