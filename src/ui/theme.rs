use gtk::gdk;
use gtk::prelude::*;

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

fn base_tokens() -> &'static str {
    r#"
        /* MITOS Design Tokens */
        @define-color mitos_radius 8px;
        @define-color mitos_font_family "Inter", "Cantarell", "Ubuntu", sans-serif;
        @define-color mitos_transition 150ms ease-in-out;
    "#
}

fn light_theme_css() -> String {
    format!(
        r#"
        {}

        /* Light Theme */
        @define-color mitos_bg #fafafa;
        @define-color mitos_surface #ffffff;
        @define-color mitos_surface_alt #f0f0f0;
        @define-color mitos_text #1a1a1a;
        @define-color mitos_text_secondary #666666;
        @define-color mitos_accent #3584e4;
        @define-color mitos_accent_hover #4a90d9;
        @define-color mitos_border #d0d0d0;
        @define-color mitos_selected #d3e3fd;
        @define-color mitos_danger #e01b24;
        @define-color mitos_success #26a269;

        window {{
            background-color: @mitos_bg;
            color: @mitos_text;
            font-family: @mitos_font_family;
        }}

        .view, listview {{
            background-color: @mitos_surface;
            color: @mitos_text;
        }}

        listview row {{
            border-radius: @mitos_radius;
            margin: 2px 4px;
        }}

        listview row:selected {{
            background-color: @mitos_selected;
        }}

        listview row:hover {{
            background-color: @mitos_surface_alt;
        }}

        button {{
            border-radius: @mitos_radius;
            transition: @mitos_transition;
        }}

        button.suggested-action {{
            background-color: @mitos_accent;
            color: white;
        }}

        button.suggested-action:hover {{
            background-color: @mitos_accent_hover;
        }}

        button.destructive-action {{
            background-color: @mitos_danger;
            color: white;
        }}

        entry {{
            border-radius: @mitos_radius;
            border: 1px solid @mitos_border;
            padding: 6px 10px;
        }}

        entry:focus-within {{
            border-color: @mitos_accent;
            box-shadow: 0 0 0 2px alpha(@mitos_accent, 0.25);
        }}

        scrollbar slider {{
            border-radius: 4px;
            min-width: 8px;
            min-height: 8px;
        }}

        .heading {{
            font-weight: bold;
            font-size: 1.1em;
        }}

        .dim-label {{
            color: @mitos_text_secondary;
        }}

        popover {{
            border-radius: @mitos_radius;
            background-color: @mitos_surface;
            box-shadow: 0 4px 16px alpha(black, 0.15);
        }}

        popover button {{
            border-radius: @mitos_radius;
        }}

        .toolbar {{
            background-color: @mitos_bg;
            border-bottom: 1px solid @mitos_border;
            padding: 6px;
        }}

        .sidebar {{
            background-color: @mitos_bg;
            border-right: 1px solid @mitos_border;
        }}

        .status-bar {{
            background-color: @mitos_bg;
            border-top: 1px solid @mitos_border;
            padding: 4px 8px;
            font-size: 0.9em;
            color: @mitos_text_secondary;
        }}

        progressbar trough {{
            border-radius: 4px;
            background-color: @mitos_surface_alt;
        }}

        progressbar progress {{
            border-radius: 4px;
            background-color: @mitos_accent;
        }}

        switch {{
            border-radius: 12px;
        }}

        switch:checked {{
            background-color: @mitos_accent;
        }}
        "#,
        base_tokens()
    )
}

fn dark_theme_css() -> String {
    format!(
        r#"
        {}

        /* Dark Theme */
        @define-color mitos_bg #1e1e1e;
        @define-color mitos_surface #2a2a2a;
        @define-color mitos_surface_alt #333333;
        @define-color mitos_text #e0e0e0;
        @define-color mitos_text_secondary #999999;
        @define-color mitos_accent #78aeed;
        @define-color mitos_accent_hover #8fbef5;
        @define-color mitos_border #404040;
        @define-color mitos_selected #2d4a6f;
        @define-color mitos_danger #ff6b6b;
        @define-color mitos_success #57d98a;

        window {{
            background-color: @mitos_bg;
            color: @mitos_text;
            font-family: @mitos_font_family;
        }}

        .view, listview {{
            background-color: @mitos_surface;
            color: @mitos_text;
        }}

        listview row {{
            border-radius: @mitos_radius;
            margin: 2px 4px;
        }}

        listview row:selected {{
            background-color: @mitos_selected;
        }}

        listview row:hover {{
            background-color: @mitos_surface_alt;
        }}

        button {{
            border-radius: @mitos_radius;
            transition: @mitos_transition;
        }}

        button.suggested-action {{
            background-color: @mitos_accent;
            color: #1a1a1a;
        }}

        button.suggested-action:hover {{
            background-color: @mitos_accent_hover;
        }}

        button.destructive-action {{
            background-color: @mitos_danger;
            color: #1a1a1a;
        }}

        entry {{
            border-radius: @mitos_radius;
            border: 1px solid @mitos_border;
            padding: 6px 10px;
            background-color: @mitos_surface_alt;
            color: @mitos_text;
        }}

        entry:focus-within {{
            border-color: @mitos_accent;
            box-shadow: 0 0 0 2px alpha(@mitos_accent, 0.25);
        }}

        scrollbar slider {{
            border-radius: 4px;
            min-width: 8px;
            min-height: 8px;
        }}

        .heading {{
            font-weight: bold;
            font-size: 1.1em;
        }}

        .dim-label {{
            color: @mitos_text_secondary;
        }}

        popover {{
            border-radius: @mitos_radius;
            background-color: @mitos_surface;
            box-shadow: 0 4px 16px alpha(black, 0.4);
        }}

        popover button {{
            border-radius: @mitos_radius;
        }}

        .toolbar {{
            background-color: @mitos_bg;
            border-bottom: 1px solid @mitos_border;
            padding: 6px;
        }}

        .sidebar {{
            background-color: @mitos_bg;
            border-right: 1px solid @mitos_border;
        }}

        .status-bar {{
            background-color: @mitos_bg;
            border-top: 1px solid @mitos_border;
            padding: 4px 8px;
            font-size: 0.9em;
            color: @mitos_text_secondary;
        }}

        progressbar trough {{
            border-radius: 4px;
            background-color: @mitos_surface_alt;
        }}

        progressbar progress {{
            border-radius: 4px;
            background-color: @mitos_accent;
        }}

        switch {{
            border-radius: 12px;
        }}

        switch:checked {{
            background-color: @mitos_accent;
        }}
        "#,
        base_tokens()
    )
}
