#![allow(dead_code)]
use hydrogen::plugins::{host::PluginHost, manager};
use std::path::Path;
pub fn plugin_host(root: &Path) -> PluginHost {
    std::fs::create_dir_all(root).unwrap();
    manager::initialize(root).unwrap();
    let host = PluginHost::new(tokio::runtime::Handle::current(), Vec::new());
    host.set_plugins(manager::discover(root, &[]).unwrap());
    host
}
