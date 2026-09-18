use gtk::gdk;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ThemeMode {
    Light,
    Dark,
}

impl ThemeMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            ThemeMode::Light => "light",
            ThemeMode::Dark => "dark",
        }
    }

    pub fn from_str(s: &str) -> Self {
        match s {
            "dark" => ThemeMode::Dark,
            _ => ThemeMode::Light,
        }
    }
}

pub fn apply_theme(display: &gdk::Display, mode: ThemeMode) {
    let provider = gtk::CssProvider::new();

    let css = match mode {
        ThemeMode::Light => light_theme_css(),
        ThemeMode::Dark => dark_theme_css(),
    };

    provider.load_from_string(&css);

    gtk::style_context_add_provider_for_display(
        display,
        &provider,
        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );
}

fn light_theme_css() -> String {
    let mut css = String::from(LIGHT_PALETTE);
    css.push_str(SHARED_CSS);
    css
}

fn dark_theme_css() -> String {
    let mut css = String::from(DARK_PALETTE);
    css.push_str(SHARED_CSS);
    css
}

// "Liquid Glass" palette -- light mode: frosted, bright glass. Translucent
// white panels over a pale blue-grey base; accents pulled darker than the
// dark palette's so cyan/violet stay readable on a light background
// instead of washing out the way neon-on-white does.
const LIGHT_PALETTE: &str = r#"
@define-color mitos_bg #eef3fb;
@define-color mitos_bg_deep #dde7f5;
@define-color mitos_surface rgba(255, 255, 255, 0.68);
@define-color mitos_surface_alt rgba(255, 255, 255, 0.92);
@define-color mitos_text #0f1b2d;
@define-color mitos_text_secondary #51617a;
@define-color mitos_accent #0891b2;
@define-color mitos_accent_hover #0aa8cf;
@define-color mitos_accent2 #8b5cf6;
@define-color mitos_border rgba(8, 145, 178, 0.28);
@define-color mitos_glow rgba(8, 145, 178, 0.35);
@define-color mitos_selected rgba(8, 145, 178, 0.14);
@define-color mitos_danger #e0284f;
@define-color mitos_success #0f9d6c;
"#;

// "Liquid Glass" palette -- dark mode: the full sci-fi read. Near-black void
// base, glass panels a shade lighter and translucent, electric cyan as the
// primary "energy" accent with violet as a secondary flourish.
const DARK_PALETTE: &str = r#"
@define-color mitos_bg #05070d;
@define-color mitos_bg_deep #000103;
@define-color mitos_surface rgba(18, 24, 38, 0.72);
@define-color mitos_surface_alt rgba(32, 42, 64, 0.88);
@define-color mitos_text #e8f2ff;
@define-color mitos_text_secondary #8fa3c4;
@define-color mitos_accent #2fe6ff;
@define-color mitos_accent_hover #7bf1ff;
@define-color mitos_accent2 #b06bff;
@define-color mitos_border rgba(47, 230, 255, 0.32);
@define-color mitos_glow rgba(47, 230, 255, 0.55);
@define-color mitos_selected rgba(47, 230, 255, 0.16);
@define-color mitos_danger #ff5470;
@define-color mitos_success #3dffb0;
"#;

// Structure, motion and the two keyframe animations -- identical between
// light and dark, written against the `@mitos_*` names each palette above
// defines.
//
// Radius/font-family/transition-timing used to be routed through
// `@define-color` (`mitos_radius`, `mitos_font_family`, `mitos_transition`)
// -- but `@define-color` only ever registers a *color*; `8px` and a font
// stack aren't one, so those three silently failed to parse and every
// `border-radius: @mitos_radius` in the old theme was falling back to
// GTK's default (square corners) instead of the intended 8px. Fixed here
// by writing the literal values directly instead of through a color
// token. This block is also a plain `&str`, not a `format!` template, on
// purpose -- nothing here needs Rust-level substitution, so there's no
// brace-escaping to get wrong either.
//
// Animation is deliberately rationed to two spots -- the selected-row glow
// and the progress-bar fill -- instead of applied broadly. A continuously
// repainting `@keyframes` on every row, or on the whole window background,
// costs real CPU/GPU for as long as the window is open, which fights the
// "faster, less memory" goal rather than serving it.
//
// `gridview child` / `columnview row` are included alongside `listview
// row` for the hover/selection rules without full certainty that's
// GtkGridView's actual CSS node name for its items (vs. also being "row")
// -- unverifiable without running this against a real display. Harmless
// if wrong (an unmatched selector is just ignored, same as an unknown
// property), but worth checking first if the grid view's cards don't pick
// up the glass hover/selection treatment.
const SHARED_CSS: &str = r#"
@keyframes mitos-pulse-glow {
    0% { box-shadow: 0 0 0 1px @mitos_glow, 0 0 10px 1px alpha(@mitos_accent, 0.35); }
    50% { box-shadow: 0 0 0 1px @mitos_glow, 0 0 18px 4px alpha(@mitos_accent, 0.55); }
    100% { box-shadow: 0 0 0 1px @mitos_glow, 0 0 10px 1px alpha(@mitos_accent, 0.35); }
}

@keyframes mitos-progress-glow {
    0% { box-shadow: 0 0 6px 1px alpha(@mitos_accent, 0.5); }
    50% { box-shadow: 0 0 12px 2px alpha(@mitos_accent, 0.8); }
    100% { box-shadow: 0 0 6px 1px alpha(@mitos_accent, 0.5); }
}

window {
    background-color: @mitos_bg;
    background-image: linear-gradient(160deg, @mitos_bg 0%, @mitos_bg_deep 100%);
    color: @mitos_text;
    font-family: "Inter", "Cantarell", "Ubuntu", sans-serif;
}

.view, listview, gridview, columnview {
    background-color: transparent;
    color: @mitos_text;
}

listview row, gridview child, columnview row {
    border: 1px solid transparent;
    border-radius: 14px;
    margin: 2px 4px;
    transition: background-color 220ms cubic-bezier(0.2, 0.8, 0.2, 1),
        box-shadow 220ms cubic-bezier(0.2, 0.8, 0.2, 1),
        border-color 220ms cubic-bezier(0.2, 0.8, 0.2, 1);
}

listview row:hover, gridview child:hover, columnview row:hover {
    background-color: @mitos_surface_alt;
    border-color: alpha(@mitos_accent, 0.25);
}

listview row:selected, gridview child:selected, columnview row:selected {
    background-color: @mitos_selected;
    border: 1px solid @mitos_glow;
    animation: mitos-pulse-glow 2.4s ease-in-out infinite;
}

button {
    border: 1px solid transparent;
    border-radius: 12px;
    transition: background-color 220ms cubic-bezier(0.2, 0.8, 0.2, 1),
        box-shadow 220ms cubic-bezier(0.2, 0.8, 0.2, 1);
}

button:hover {
    box-shadow: 0 0 0 1px alpha(@mitos_accent_hover, 0.35);
}

button.suggested-action {
    background-image: linear-gradient(135deg, @mitos_accent 0%, @mitos_accent2 100%);
    border: none;
    color: #04121a;
}

button.suggested-action:hover {
    box-shadow: 0 0 14px 1px alpha(@mitos_accent, 0.6);
}

button.destructive-action {
    background-color: @mitos_danger;
    border: none;
    color: #ffffff;
}

entry {
    background-color: @mitos_surface;
    border: 1px solid @mitos_border;
    border-radius: 12px;
    color: @mitos_text;
    padding: 6px 10px;
}

entry:focus-within {
    border-color: @mitos_accent;
    box-shadow: 0 0 0 2px alpha(@mitos_accent, 0.25);
}

scrollbar slider {
    background-color: alpha(@mitos_accent, 0.4);
    border-radius: 999px;
    min-width: 8px;
    min-height: 8px;
}

scrollbar slider:hover {
    background-color: @mitos_accent_hover;
}

checkbutton check, radiobutton radio {
    border: 1px solid @mitos_border;
    border-radius: 6px;
}

checkbutton check:checked, radiobutton radio:checked {
    background-color: @mitos_accent;
    border-color: @mitos_accent;
}

.heading {
    font-size: 1.1em;
    font-weight: 600;
    letter-spacing: 0.02em;
}

.dim-label {
    color: @mitos_text_secondary;
}

.success {
    color: @mitos_success;
}

.error, .danger {
    color: @mitos_danger;
}

entry.error {
    border-color: @mitos_danger;
    box-shadow: 0 0 0 2px alpha(@mitos_danger, 0.25);
}

popover {
    background-color: @mitos_surface_alt;
    border: 1px solid @mitos_border;
    border-radius: 16px;
    box-shadow: 0 8px 32px alpha(black, 0.35), 0 0 0 1px alpha(@mitos_accent, 0.08);
}

popover button {
    border-radius: 10px;
}

.toolbar {
    background-color: @mitos_surface;
    border-bottom: 1px solid @mitos_border;
    padding: 6px;
}

.sidebar {
    background-color: @mitos_surface;
    border-right: 1px solid @mitos_border;
}

.sidebar row:selected {
    background-color: @mitos_selected;
    border-left: 2px solid @mitos_accent;
}

.status-bar {
    background-color: @mitos_surface;
    border-top: 1px solid @mitos_border;
    color: @mitos_text_secondary;
    font-size: 0.9em;
    padding: 5px 10px;
}

progressbar trough {
    background-color: @mitos_surface_alt;
    border-radius: 999px;
    min-height: 6px;
}

progressbar progress {
    animation: mitos-progress-glow 1.6s ease-in-out infinite;
    background-image: linear-gradient(90deg, @mitos_accent2 0%, @mitos_accent 100%);
    border-radius: 999px;
}

switch {
    background-color: @mitos_surface_alt;
    border: 1px solid @mitos_border;
    border-radius: 999px;
}

switch:checked {
    background-image: linear-gradient(135deg, @mitos_accent 0%, @mitos_accent2 100%);
}

switch slider {
    border-radius: 999px;
}

notebook > header {
    background-color: @mitos_surface;
    border-bottom: 1px solid @mitos_border;
}

notebook > header tab {
    border-radius: 10px 10px 0 0;
    transition: background-color 220ms cubic-bezier(0.2, 0.8, 0.2, 1);
}

notebook > header tab:checked {
    background-color: @mitos_surface_alt;
    box-shadow: inset 0 -2px 0 0 @mitos_accent;
}
"#;
