rust_i18n::i18n!("locales", fallback = "en");

pub mod audio;
pub mod backend;
pub mod controller;
pub mod demo;
pub mod i18n;
pub mod model;
pub mod os;
pub mod player;
pub mod playlist_index;
pub mod recent;
pub mod search;
pub mod sort;
pub mod store;
#[cfg(test)]
pub mod test_support;
pub mod tidal;
pub mod ui;
