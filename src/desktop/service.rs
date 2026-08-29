use std::sync::mpsc;

pub enum DesktopRequest {
    SetWallpaper {
        path: String,
        response_tx: mpsc::Sender<Result<(), String>>,
    },
    GetWallpaper {
        response_tx: mpsc::Sender<String>,
    },
    OpenDesktopSettings {
        response_tx: mpsc::Sender<Result<(), String>>,
    },
}

pub fn start() -> mpsc::Receiver<DesktopRequest> {
    let (request_tx, request_rx) = mpsc::channel();

    std::thread::spawn(move || {
        if let Err(err) = run_dbus_service(request_tx) {
            eprintln!("Desktop D-Bus service error: {}", err);
        }
    });

    request_rx
}

fn run_dbus_service(
    request_tx: mpsc::Sender<DesktopRequest>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    use zbus::blocking::Connection;
    use zbus::interface;

    struct MitosDesktop {
        request_tx: mpsc::Sender<DesktopRequest>,
    }

    #[interface(name = "org.mitos.Desktop")]
    impl MitosDesktop {
        fn set_wallpaper(&self, path: &str) -> zbus::fdo::Result<()> {
            let (response_tx, response_rx) = mpsc::channel();

            self.request_tx
                .send(DesktopRequest::SetWallpaper {
                    path: path.to_string(),
                    response_tx,
                })
                .map_err(|e| zbus::fdo::Error::Failed(format!("Channel error: {}", e)))?;

            match response_rx.recv() {
                Ok(Ok(())) => Ok(()),
                Ok(Err(msg)) => Err(zbus::fdo::Error::Failed(msg)),
                Err(e) => Err(zbus::fdo::Error::Failed(format!("Response error: {}", e))),
            }
        }

        fn get_wallpaper(&self) -> zbus::fdo::Result<String> {
            let (response_tx, response_rx) = mpsc::channel();

            self.request_tx
                .send(DesktopRequest::GetWallpaper { response_tx })
                .map_err(|e| zbus::fdo::Error::Failed(format!("Channel error: {}", e)))?;

            match response_rx.recv() {
                Ok(path) => Ok(path),
                Err(e) => Err(zbus::fdo::Error::Failed(format!("Response error: {}", e))),
            }
        }
    }

    let connection = Connection::session()?;

    connection.request_name("org.mitos.Desktop")?;

    let desktop = MitosDesktop { request_tx };

    connection
        .object_server()
        .at("/org/mitos/Desktop", desktop)?;

    loop {
        std::thread::sleep(std::time::Duration::from_secs(1));
    }
}
