#[derive(Clone, Debug)]
pub struct SearchFilters {
    pub query: String,
    pub recursive: bool,
    pub match_file_name: bool,
    /// Also look inside text files for `query` (ASCII case-insensitive).
    pub match_content: bool,
    /// Whether dotfiles / dot-folders are searched. Follows the tab's
    /// "Hidden" toggle, so searching your home folder doesn't crawl
    /// `~/.cache` unless you asked to see hidden files.
    pub include_hidden: bool,
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
            include_hidden: false,
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
    /// Every filter, in the order the search bar's type dropdown lists them.
    pub fn all() -> [FileTypeFilter; 7] {
        [
            FileTypeFilter::Images,
            FileTypeFilter::Videos,
            FileTypeFilter::Audio,
            FileTypeFilter::Documents,
            FileTypeFilter::Archives,
            FileTypeFilter::Code,
            FileTypeFilter::Folders,
        ]
    }

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_filter_is_listed_once_with_its_own_label() {
        let all = FileTypeFilter::all();
        let mut labels: Vec<&str> = all.iter().map(|filter| filter.label()).collect();

        assert_eq!(labels.len(), 7);

        labels.sort();
        labels.dedup();
        assert_eq!(labels.len(), 7);
    }

    #[test]
    fn filters_match_the_mime_types_they_claim_to() {
        assert!(FileTypeFilter::Images.matches_mime("image/png"));
        assert!(FileTypeFilter::Videos.matches_mime("video/mp4"));
        assert!(FileTypeFilter::Audio.matches_mime("audio/mpeg"));
        assert!(FileTypeFilter::Documents.matches_mime("application/pdf"));
        assert!(FileTypeFilter::Documents.matches_mime("text/plain"));
        assert!(FileTypeFilter::Archives.matches_mime("application/zip"));
        assert!(FileTypeFilter::Code.matches_mime("application/json"));
        assert!(FileTypeFilter::Folders.matches_mime("inode/directory"));

        assert!(!FileTypeFilter::Images.matches_mime("video/mp4"));
        assert!(!FileTypeFilter::Folders.matches_mime("text/plain"));
    }
}
