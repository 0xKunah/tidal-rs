//! UI language selection.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Lang {
	En,
	Fr,
}

impl Lang {
	/// Locale code used by the translation files.
	pub fn code(self) -> &'static str {
		match self {
			Lang::En => "en",
			Lang::Fr => "fr",
		}
	}
}

/// The setting wins; otherwise the Windows language when supported; otherwise English.
pub fn resolve(setting: Option<Lang>, system: Option<&str>) -> Lang {
	if let Some(lang) = setting {
		return lang;
	}
	match system {
		Some(locale) if locale.to_ascii_lowercase().starts_with("fr") => Lang::Fr,
		_ => Lang::En,
	}
}

/// Switches every translated string, including gpui-component's own.
pub fn apply(lang: Lang) {
	rust_i18n::set_locale(lang.code());
}

pub fn system_locale() -> Option<String> {
	sys_locale::get_locale()
}

#[cfg(test)]
mod tests {
	use std::collections::BTreeSet;

	use super::*;

	#[test]
	fn setting_wins() {
		assert_eq!(resolve(Some(Lang::Fr), Some("en-US")), Lang::Fr);
	}

	#[test]
	fn follows_windows_language() {
		assert_eq!(resolve(None, Some("fr-FR")), Lang::Fr);
	}

	#[test]
	fn unknown_or_missing_falls_back_to_english() {
		assert_eq!(resolve(None, Some("de-DE")), Lang::En);
		assert_eq!(resolve(None, None), Lang::En);
	}

	fn keys(text: &str) -> BTreeSet<String> {
		fn walk(prefix: &str, value: &toml::Value, out: &mut BTreeSet<String>) {
			match value {
				toml::Value::Table(table) => {
					for (key, child) in table {
						let path = if prefix.is_empty() {
							key.clone()
						} else {
							format!("{prefix}.{key}")
						};
						walk(&path, child, out);
					}
				}
				_ => {
					out.insert(prefix.to_string());
				}
			}
		}
		let value: toml::Value = toml::from_str(text).unwrap();
		let mut out = BTreeSet::new();
		walk("", &value, &mut out);
		out
	}

	#[test]
	fn translations_load_for_both_languages() {
		assert_eq!(
			rust_i18n::t!("sidebar.recent", locale = "fr"),
			"Écoutés récemment"
		);
		assert_eq!(
			rust_i18n::t!("sidebar.recent", locale = "en"),
			"Recently played"
		);
		assert_eq!(
			rust_i18n::t!("page.track_count", locale = "fr", count = 3),
			"3 titres"
		);
	}

	#[test]
	fn locale_files_have_the_same_keys() {
		let en = keys(include_str!("../locales/en.toml"));
		let fr = keys(include_str!("../locales/fr.toml"));
		assert!(!en.is_empty());
		assert_eq!(en, fr);
	}
}
