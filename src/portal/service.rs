use std::path::PathBuf;
use std::sync::mpsc;

/// Requests sent from the D-Bus portal thread to the GTK main thread.
pub enum PortalRequest {
    OpenFile {
        title: String,
        response_tx: mpsc::Sender<PortalResponse>,
    },
    SaveFile {
        title: String,
        default_name: String,
        response_tx: mpsc::Sender<PortalResponse>,
    },
    OpenFolder {
        title: String,
        response_tx: mpsc::Sender<PortalResponse>,
    },
}

/// Responses sent from the GTK main thread back to the D-Bus portal thread.
pub enum PortalResponse {
    Selected(Vec<String>),
    Cancelled,
    Error(String),
}

/// Start the D-Bus portal service in a background thread.
/// Returns a receiver that the GTK main thread should poll for incoming requests.
pub fn start() -> mpsc::Receiver<PortalRequest> {
    let (request_tx, request_rx) = mpsc::channel();

    std::thread::spawn(move || {
        if let Err(err) = run_dbus_service(request_tx) {
            eprintln!("Portal D-Bus service error: {}", err);
        }
    });

    request_rx
}

fn run_dbus_service(
    request_tx: mpsc::Sender<PortalRequest>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    use zbus::blocking::Connection;
    use zbus::interface;

    struct FilePickerPortal {
        request_tx: mpsc::Sender<PortalRequest>,
    }

    #[interface(name = "org.mitos.FilePicker")]
    impl FilePickerPortal {
        fn open_file(&self, title: &str) -> zbus::fdo::Result<Vec<String>> {
            let (response_tx, response_rx) = mpsc::channel();

            self.request_tx
                .send(PortalRequest::OpenFile {
                    title: title.to_string(),
                    response_tx,
                })
                .map_err(|e| zbus::fdo::Error::Failed(format!("Channel error: {}", e)))?;

            match response_rx.recv() {
                Ok(PortalResponse::Selected(paths)) => Ok(paths),
                Ok(PortalResponse::Cancelled) => Ok(Vec::new()),
                Ok(PortalResponse::Error(msg)) => Err(zbus::fdo::Error::Failed(msg)),
                Err(e) => Err(zbus::fdo::Error::Failed(format!("Response error: {}", e))),
            }
        }

        fn save_file(&self, title: &str, default_name: &str) -> zbus::fdo::Result<Vec<String>> {
            let (response_tx, response_rx) = mpsc::channel();

            self.request_tx
                .send(PortalRequest::SaveFile {
                    title: title.to_string(),
                    default_name: default_name.to_string(),
                    response_tx,
                })
                .map_err(|e| zbus::fdo::Error::Failed(format!("Channel error: {}", e)))?;

            match response_rx.recv() {
                Ok(PortalResponse::Selected(paths)) => Ok(paths),
                Ok(PortalResponse::Cancelled) => Ok(Vec::new()),
                Ok(PortalResponse::Error(msg)) => Err(zbus::fdo::Error::Failed(msg)),
                Err(e) => Err(zbus::fdo::Error::Failed(format!("Response error: {}", e))),
            }
        }

        fn open_folder(&self, title: &str) -> zbus::fdo::Result<Vec<String>> {
            let (response_tx, response_rx) = mpsc::channel();

            self.request_tx
                .send(PortalRequest::OpenFolder {
                    title: title.to_string(),
                    response_tx,
                })
                .map_err(|e| zbus::fdo::Error::Failed(format!("Channel error: {}", e)))?;

            match response_rx.recv() {
                Ok(PortalResponse::Selected(paths)) => Ok(paths),
                Ok(PortalResponse::Cancelled) => Ok(Vec::new()),
                Ok(PortalResponse::Error(msg)) => Err(zbus::fdo::Error::Failed(msg)),
                Err(e) => Err(zbus::fdo::Error::Failed(format!("Response error: {}", e))),
            }
        }
    }

    let connection = Connection::session()?;

    connection.request_name("org.mitos.FilePicker")?;

    let portal = FilePickerPortal { request_tx };

    connection
        .object_server()
        .at("/org/mitos/FilePicker", portal)?;

    // Keep the service alive
    loop {
        std::thread::sleep(std::time::Duration::from_secs(1));
    }
}
