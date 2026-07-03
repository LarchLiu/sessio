use std::sync::Mutex;

#[derive(Default)]
pub(crate) struct AppshotShortcutState {
    pub(crate) registered_shortcut: Mutex<Option<String>>,
    pub(crate) suspended_shortcut: Mutex<Option<String>>,
}
