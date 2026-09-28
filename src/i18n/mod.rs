//! Multi-language UI text.
//!
//! Scope, honestly stated: this translates the app's static chrome --
//! button labels, menu items, dialog titles, tooltips, column headers,
//! empty-state text. It does **not** attempt to translate: text with
//! interpolated data (file names, counts, paths, error messages --
//! `format!("{count} items")` stays "{count} items" in every language,
//! because correct pluralization needs per-language grammar rules this
//! module doesn't implement); the batch-rename pattern tokens (`{name}`,
//! `{ext}`, ...), which are literal syntax the user types, not prose;
//! plugin-supplied labels, which are the plugin author's own text.
//!
//! How it works: every translatable string uses its own English text as
//! the lookup key (the same approach gettext uses) -- so call sites read
//! `tr("Cancel")` rather than `tr("dialog.cancel.label")`. `tr()` looks the
//! key up in the current language's table; a key with no entry (including
//! every key when the language *is* English) falls back to the key itself,
//! so the fallback is always sensible text rather than an ugly missing-key
//! marker.
//!
//! Adding a language: add a variant to `Lang`, a `catalog.rs` file with a
//! `pub const TABLE: &[(&str, &str)]` of (English, translated) pairs, and
//! wire it into `catalog_for` below. Existing call sites need no changes --
//! any English key without an entry in the new table just falls back to
//! English for that string until someone adds it.

mod ar;
mod es;
mod fr;
mod tw;

use std::collections::HashMap;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::OnceLock;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lang {
    En,
    Fr,
    Es,
    Ar,
    Tw,
}

impl Lang {
    /// Every supported language, in the order a picker should list them.
    pub const fn all() -> [Lang; 5] {
        [Lang::En, Lang::Fr, Lang::Es, Lang::Ar, Lang::Tw]
    }

    /// The short code stored in settings and matched against `$LANG` /
    /// `$LC_ALL` / etc. (ISO 639-1, lowercase).
    pub const fn code(self) -> &'static str {
        match self {
            Lang::En => "en",
            Lang::Fr => "fr",
            Lang::Es => "es",
            Lang::Ar => "ar",
            Lang::Tw => "tw",
        }
    }

    /// The language's own name for itself, for a language picker -- someone
    /// looking for their language reads it in that language, not in
    /// whatever the UI currently happens to be in.
    pub const fn native_name(self) -> &'static str {
        match self {
            Lang::En => "English",
            Lang::Fr => "Français",
            Lang::Es => "Español",
            Lang::Ar => "العربية",
            Lang::Tw => "Twi",
        }
    }

    pub fn from_code(code: &str) -> Option<Lang> {
        let code = code.to_ascii_lowercase();

        Self::all().into_iter().find(|lang| lang.code() == code)
    }

    /// Right-to-left script -- Arabic reads right to left, so callers that
    /// build directional UI (rare in this app; GTK handles ordinary label
    /// text on its own) can check this.
    pub fn is_rtl(self) -> bool {
        matches!(self, Lang::Ar)
    }
}

impl Default for Lang {
    fn default() -> Self {
        Lang::En
    }
}

/// Index into `Lang::all()` of the language `tr()` currently reads from.
/// An index (not the enum itself) so it fits an atomic; `Relaxed` is fine
/// since this is read far more often than written and every value is a
/// valid, complete language table -- there's no partial state to
/// synchronize against.
static CURRENT: AtomicU8 = AtomicU8::new(0);

pub fn current() -> Lang {
    Lang::all()
        .get(CURRENT.load(Ordering::Relaxed) as usize)
        .copied()
        .unwrap_or(Lang::En)
}

pub fn set_current(lang: Lang) {
    let index = Lang::all().iter().position(|&l| l == lang).unwrap_or(0);
    CURRENT.store(index as u8, Ordering::Relaxed);
}

/// Read `$LANGUAGE`, `$LC_ALL`, `$LC_MESSAGES`, `$LANG` in that order (the
/// same priority gettext uses) and match the leading language subtag
/// against a supported language. `$LANGUAGE` can list several separated by
/// `:` (gettext's own convention); the first supported one wins. Falls
/// back to English if nothing matches, is unset, or is "C" / "POSIX".
pub fn detect_system_language() -> Lang {
    for var in ["LANGUAGE", "LC_ALL", "LC_MESSAGES", "LANG"] {
        let Ok(value) = std::env::var(var) else {
            continue;
        };

        for candidate in value.split(':') {
            if let Some(lang) = subtag_to_lang(candidate) {
                return lang;
            }
        }
    }

    Lang::En
}

/// "fr_FR.UTF-8" / "fr-FR" / "fr" -> `Lang::Fr`. "C" / "POSIX" / empty ->
/// `None` (not a real language tag).
fn subtag_to_lang(value: &str) -> Option<Lang> {
    let primary = value
        .split(['_', '-', '.'])
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();

    if primary.is_empty() || primary == "c" || primary == "posix" {
        return None;
    }

    Lang::from_code(&primary)
}

/// Set the active language from the persisted setting: `stored_code`
/// verbatim if it names a supported language, otherwise the system's.
/// Called once at startup (`config::settings::load`).
pub fn init_from_setting(stored_code: &str) {
    let lang = Lang::from_code(stored_code).unwrap_or_else(detect_system_language);
    set_current(lang);
}

fn catalog_for(lang: Lang) -> Option<&'static HashMap<&'static str, &'static str>> {
    fn build(table: &'static [(&'static str, &'static str)]) -> HashMap<&'static str, &'static str> {
        table.iter().copied().collect()
    }

    match lang {
        Lang::En => None,
        Lang::Fr => {
            static TABLE: OnceLock<HashMap<&str, &str>> = OnceLock::new();
            Some(TABLE.get_or_init(|| build(fr::TABLE)))
        }
        Lang::Es => {
            static TABLE: OnceLock<HashMap<&str, &str>> = OnceLock::new();
            Some(TABLE.get_or_init(|| build(es::TABLE)))
        }
        Lang::Ar => {
            static TABLE: OnceLock<HashMap<&str, &str>> = OnceLock::new();
            Some(TABLE.get_or_init(|| build(ar::TABLE)))
        }
        Lang::Tw => {
            static TABLE: OnceLock<HashMap<&str, &str>> = OnceLock::new();
            Some(TABLE.get_or_init(|| build(tw::TABLE)))
        }
    }
}

/// Translate a static UI string. `key` is the English text (also what's
/// shown when the current language is English, or has no entry for it).
///
/// Cheap to call on every widget build: English (the common case during
/// development and for anyone who hasn't changed the setting) skips the
/// lookup entirely, and every other language is one hash-map get.
pub fn tr(key: &str) -> String {
    match catalog_for(current()).and_then(|table| table.get(key)) {
        Some(translated) => translated.to_string(),
        None => key.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Tests that read/set the global current-language must not run
    // concurrently with each other (they'd race), so each restores it
    // when done rather than relying on test order.
    fn with_lang<R>(lang: Lang, body: impl FnOnce() -> R) -> R {
        let previous = current();
        set_current(lang);
        let result = body();
        set_current(previous);
        result
    }

    #[test]
    fn english_is_the_fallback_for_every_language() {
        with_lang(Lang::Fr, || {
            assert_eq!(tr("Some string nobody has translated yet"), "Some string nobody has translated yet");
        });
    }

    #[test]
    fn english_never_touches_the_catalog() {
        with_lang(Lang::En, || {
            assert_eq!(tr("Cancel"), "Cancel");
        });
    }

    #[test]
    fn a_translated_key_returns_the_translation() {
        with_lang(Lang::Fr, || {
            assert_eq!(tr("Cancel"), "Annuler");
        });
        with_lang(Lang::Es, || {
            assert_eq!(tr("Cancel"), "Cancelar");
        });
    }

    #[test]
    fn every_language_round_trips_through_its_code() {
        for lang in Lang::all() {
            assert_eq!(Lang::from_code(lang.code()), Some(lang));
        }

        assert_eq!(Lang::from_code("xx"), None);
        assert_eq!(Lang::from_code("FR"), Some(Lang::Fr));
    }

    #[test]
    fn locale_strings_resolve_to_the_right_language() {
        assert_eq!(subtag_to_lang("fr_FR.UTF-8"), Some(Lang::Fr));
        assert_eq!(subtag_to_lang("es-MX"), Some(Lang::Es));
        assert_eq!(subtag_to_lang("ar"), Some(Lang::Ar));
        assert_eq!(subtag_to_lang("C"), None);
        assert_eq!(subtag_to_lang("POSIX"), None);
        assert_eq!(subtag_to_lang(""), None);
        assert_eq!(subtag_to_lang("de_DE"), None); // not (yet) supported
    }

    #[test]
    fn only_arabic_is_marked_right_to_left() {
        assert!(Lang::Ar.is_rtl());
        assert!(!Lang::En.is_rtl());
        assert!(!Lang::Tw.is_rtl());
    }

    #[test]
    fn set_current_and_current_agree() {
        with_lang(Lang::Es, || {
            assert_eq!(current(), Lang::Es);
        });
    }
}
