/* This is largely copied from the typst-cli main since the font searching is
   not exposed by the typst library. */

use std::path::{ Path, PathBuf };
use typst::text::{ FontBook, FontInfo };
use memmap2::Mmap;
use walkdir::WalkDir;

/// Holds details about the location of a font and lazily the font itself.
pub struct FontSlot {
    pub path: PathBuf,
    pub index: u32,
}

/// Searches for fonts.
pub struct FontSearcher {
    pub book: FontBook,
    pub fonts: Vec<FontSlot>,
}

impl FontSearcher {
    /// Create a new, empty system searcher.
    pub fn new() -> Self {
        Self { book: FontBook::new(), fonts: vec![] }
    }

    /// Search for fonts in the linux system font directories.
    #[cfg(all(unix, not(target_os = "macos")))]
    pub fn search_system(&mut self) {
        self.search_dir("/usr/share/fonts");
        self.search_dir("/usr/local/share/fonts");

        if let Some(dir) = dirs::font_dir() {
            self.search_dir(dir);
        }
    }

    /// Search for fonts in the macOS system font directories.
    #[cfg(target_os = "macos")]
    pub fn search_system(&mut self) {
        self.search_dir("/Library/Fonts");
        self.search_dir("/Network/Library/Fonts");
        self.search_dir("/System/Library/Fonts");

        if let Some(dir) = dirs::font_dir() {
            self.search_dir(dir);
        }
    }

    /// Search for fonts in the Windows system font directories.
    #[cfg(windows)]
    pub fn search_system(&mut self) {
        let windir =
            std::env::var("WINDIR").unwrap_or_else(|_| "C:\\Windows".to_string());

        self.search_dir(Path::new(&windir).join("Fonts"));

        if let Some(roaming) = dirs::config_dir() {
            self.search_dir(roaming.join("Microsoft\\Windows\\Fonts"));
        }

        if let Some(local) = dirs::cache_dir() {
            self.search_dir(local.join("Microsoft\\Windows\\Fonts"));
        }
    }

    /// Search for all fonts in a directory recursively.
    pub fn search_dir(&mut self, path: impl AsRef<Path>) {
        for entry in WalkDir::new(path)
            .follow_links(true)
            .sort_by(|a, b| a.file_name().cmp(b.file_name()))
            .into_iter()
            .filter_map(|e| e.ok())
        {
            let path = entry.path();
            if matches!(
                path.extension().and_then(|s| s.to_str()),
                Some("ttf" | "otf" | "TTF" | "OTF" | "ttc" | "otc" | "TTC" | "OTC"),
            ) {
                self.search_file(path);
            }
        }
    }

    /// Index the fonts in the file at the given path.
    pub fn search_file(&mut self, path: impl AsRef<Path>) {
        let path = path.as_ref();
        if let Ok(file) = std::fs::File::open(path) {
            if let Ok(mmap) = unsafe { Mmap::map(&file) } {
                for (i, info) in FontInfo::iter(&mmap).enumerate() {
                    self.book.push(info);
                    self.fonts.push(FontSlot {
                        path: path.into(),
                        index: i as u32,
                    });
                }
            }
        }
    }
}
