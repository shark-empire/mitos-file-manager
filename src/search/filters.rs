#[derive(Clone, Debug)]
pub struct SearchFilters {
    pub query: String,
    pub recursive: bool,
    pub match_file_name: bool,
    pub match_content: bool,
    pub file_types: Vec<FileTypeFilter>,
    pub min_size_bytes: Option<u64>,
    pub max_size_bytes: Option<u64>,
}

impl Default for SearchFilters {
    fn default() -> Self {
        Self {
            query: String::new(),
            recursive: true,
            match_file_name: true,
            match_content: false,
            file_types: Vec::new(),
            min_size_bytes: None,
            max_size_bytes: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum FileTypeFilter {
    Images,
    Videos,
    Audio,
    Documents,
    Archives,
    Code,
    Folders,
}

impl FileTypeFilter {
    pub fn matches_mime(&self, mime: &str) -> bool {
        match self {
            FileTypeFilter::Images => mime.starts_with("image/"),
            FileTypeFilter::Videos => mime.starts_with("video/"),
            FileTypeFilter::Audio => mime.starts_with("audio/"),
            FileTypeFilter::Documents => {
                mime.starts_with("text/")
                    || mime.contains("pdf")
                    || mime.contains("document")
                    || mime.contains("spreadsheet")
                    || mime.contains("presentation")
            }
            FileTypeFilter::Archives => {
                mime.contains("zip")
                    || mime.contains("tar")
                    || mime.contains("compressed")
                    || mime.contains("archive")
            }
            FileTypeFilter::Code => {
                mime.contains("script")
                    || mime.contains("source")
                    || mime.contains("programming")
                    || mime.contains("json")
                    || mime.contains("xml")
                    || mime.contains("html")
                    || mime.contains("css")
            }
            FileTypeFilter::Folders => mime == "inode/directory",
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            FileTypeFilter::Images => "Images",
            FileTypeFilter::Videos => "Videos",
            FileTypeFilter::Audio => "Audio",
            FileTypeFilter::Documents => "Documents",
            FileTypeFilter::Archives => "Archives",
            FileTypeFilter::Code => "Code",
            FileTypeFilter::Folders => "Folders",
        }
    }
}
