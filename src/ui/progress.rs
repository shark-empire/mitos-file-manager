use crate::filesystem::metadata;
use crate::operations::jobs::{JobHandle, JobMessage};
use gtk::glib;
use gtk::prelude::*;
use gtk::{ApplicationWindow, Box as GtkBox, Button, Label, Orientation, ProgressBar};
use std::cell::Cell;
use std::rc::Rc;
use std::sync::atomic::Ordering;
use std::time::Instant;

pub fn show_progress_dialog<F>(
    parent: &ApplicationWindow,
    title: &str,
    handle: JobHandle,
    receiver: glib::Receiver<JobMessage>,
    on_done: F,
) where
    F: Fn(Result<usize, String>) + 'static,
{
    let window = gtk::Window::builder()
        .title(title)
        .transient_for(parent)
        .modal(false)
        .default_width(480)
        .build();

    let vbox = GtkBox::new(Orientation::Vertical, 8);

    vbox.set_margin_top(12);
    vbox.set_margin_bottom(12);
    vbox.set_margin_start(12);
    vbox.set_margin_end(12);

    let label = Label::new(Some("Preparing..."));
    label.set_halign(gtk::Align::Start);

    let bar = ProgressBar::new();
    bar.set_show_text(true);

    let controls = GtkBox::new(Orientation::Horizontal, 8);

    let pause_btn = Button::with_label("Pause");
    let cancel_btn = Button::with_label("Cancel");

    controls.append(&pause_btn);
    controls.append(&cancel_btn);

    vbox.append(&label);
    vbox.append(&bar);
    vbox.append(&controls);

    window.set_child(Some(&vbox));

    let closed = Rc::new(Cell::new(false));

    {
        let pause = handle.pause.clone();

        pause_btn.connect_clicked(move |btn| {
            let paused = !pause.load(Ordering::Relaxed);
            pause.store(paused, Ordering::Relaxed);

            if paused {
                btn.set_label("Resume");
            } else {
                btn.set_label("Pause");
            }
        });
    }

    {
        let cancel = handle.cancel.clone();

        cancel_btn.connect_clicked(move |_| {
            cancel.store(true, Ordering::Relaxed);
        });
    }

    {
        let cancel = handle.cancel.clone();
        let closed = closed.clone();

        window.connect_close_request(move |_| {
            cancel.store(true, Ordering::Relaxed);
            closed.set(true);
            gtk::Inhibit(false)
        });
    }

    let start = Instant::now();

    receiver.attach(None, move |message| {
        if closed.get() {
            return glib::ControlFlow::Break;
        }

        match message {
            JobMessage::Started {
                label: text,
                total,
                bytes: _,
            } => {
                label.set_label(&text);

                if total == 0 {
                    bar.pulse();
                } else {
                    bar.set_fraction(0.0);
                }
            }

            JobMessage::Progress {
                label: text,
                processed,
                total,
                bytes,
            } => {
                label.set_label(&text);

                if total == 0 {
                    bar.pulse();
                } else {
                    bar.set_fraction((processed as f64 / total as f64).clamp(0.0, 1.0));
                }

                let detail = if bytes {
                    let elapsed = start.elapsed().as_secs_f64();

                    let speed = if elapsed > 0.0 {
                        processed as f64 / elapsed
                    } else {
                        0.0
                    };

                    let remaining = if speed > 0.0 && total > processed {
                        (total - processed) as f64 / speed
                    } else {
                        0.0
                    };

                    format!(
                        "{} of {} · {}/s · {} left",
                        metadata::format_size(processed),
                        metadata::format_size(total),
                        metadata::format_size(speed as u64),
                        format_duration(remaining)
                    )
                } else {
                    format!("{} of {} items", processed, total)
                };

                bar.set_text(Some(&detail));
            }

            JobMessage::Finished { result } => {
                window.close();
                on_done(result);
                return glib::ControlFlow::Break;
            }
        }

        glib::ControlFlow::Continue
    });

    window.present();
}

fn format_duration(secs: f64) -> String {
    let secs = secs.max(0.0) as u64;

    if secs < 60 {
        format!("{secs}s")
    } else if secs < 3600 {
        format!("{}m {}s", secs / 60, secs % 60)
    } else {
        format!("{}h {}m", secs / 3600, (secs % 3600) / 60)
    }
}
