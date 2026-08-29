use gtk::prelude::*;
use gtk::{Application, ApplicationWindow};

pub fn create_window(app: &Application, title: &str) -> ApplicationWindow {
    ApplicationWindow::builder()
        .application(app)
        .title(title)
        .default_width(1100)
        .default_height(720)
        .build()
}
