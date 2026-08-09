//! Game asset lookup — mirrors TS `asset-catalog.ts`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

fn normalized(value: &str) -> String {
    value
        .to_lowercase()
        .chars()
        .filter(|c| c.is_alphanumeric())
        .collect()
}

fn is_image_ext(path: &Path) -> bool {
    matches!(path.extension().and_then(|e| e.to_str()).map(|e| e.to_lowercase()),
        Some(ref ext) if ext == "png" || ext == "webp" || ext == "jpg" || ext == "jpeg" || ext == "avif")
}

#[derive(Clone)]
pub struct AssetCatalog {
    root: PathBuf,
    champion_files: Arc<RwLock<Option<Vec<PathBuf>>>>,
    map_files: Arc<RwLock<Option<Vec<PathBuf>>>>,
    rank_files: Arc<RwLock<Option<Vec<PathBuf>>>>,
    icon_files: Arc<RwLock<Option<Vec<PathBuf>>>>,
    champion_icons: Arc<RwLock<HashMap<String, Option<PathBuf>>>>,
    champion_banners: Arc<RwLock<HashMap<String, Option<PathBuf>>>>,
    talent_icons: Arc<RwLock<HashMap<String, Option<PathBuf>>>>,
    map_images: Arc<RwLock<HashMap<String, Option<PathBuf>>>>,
    rank_icons: Arc<RwLock<HashMap<u32, Option<PathBuf>>>>,
    icons: Arc<RwLock<HashMap<String, Option<PathBuf>>>>,
}

impl AssetCatalog {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            champion_files: Arc::new(RwLock::new(None)),
            map_files: Arc::new(RwLock::new(None)),
            rank_files: Arc::new(RwLock::new(None)),
            icon_files: Arc::new(RwLock::new(None)),
            champion_icons: Arc::new(RwLock::new(HashMap::new())),
            champion_banners: Arc::new(RwLock::new(HashMap::new())),
            talent_icons: Arc::new(RwLock::new(HashMap::new())),
            map_images: Arc::new(RwLock::new(HashMap::new())),
            rank_icons: Arc::new(RwLock::new(HashMap::new())),
            icons: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub fn champion_icon(&self, champion_name: &str) -> Option<PathBuf> {
        let key = normalized(champion_name);
        if let Some(v) = self.champion_icons.read().unwrap().get(&key) {
            return v.clone();
        }
        let files = self.load_champion_files();
        let wanted = normalized(&format!("champion {} icon", champion_name));
        let result = files
            .iter()
            .find(|f| {
                f.file_stem()
                    .map(|s| normalized(&s.to_string_lossy()) == wanted)
                    .unwrap_or(false)
            })
            .or_else(|| {
                files.iter().find(|f| {
                    let n = normalized(&f.to_string_lossy());
                    n.contains(&normalized(champion_name)) && n.contains("icon")
                })
            })
            .cloned();
        self.champion_icons
            .write()
            .unwrap()
            .insert(key, result.clone());
        result
    }

    pub fn champion_banner(&self, champion_name: &str) -> Option<PathBuf> {
        let key = normalized(champion_name);
        if let Some(v) = self.champion_banners.read().unwrap().get(&key) {
            return v.clone();
        }
        let files = self.load_champion_files();
        let wanted = normalized(&format!("banner {}", champion_name));
        let result = files
            .iter()
            .filter(|f| {
                f.file_stem()
                    .map(|s| normalized(&s.to_string_lossy()) == wanted)
                    .unwrap_or(false)
            })
            .find(|f| f.extension().map(|e| e == "png").unwrap_or(false))
            .or_else(|| {
                files.iter().find(|f| {
                    f.file_stem()
                        .map(|s| normalized(&s.to_string_lossy()) == wanted)
                        .unwrap_or(false)
                })
            })
            .cloned()
            .or_else(|| self.champion_icon(champion_name));
        self.champion_banners
            .write()
            .unwrap()
            .insert(key, result.clone());
        result
    }

    pub fn talent_icon(
        &self,
        talent_id: Option<u32>,
        champion_name: &str,
        talent_name: &str,
    ) -> Option<PathBuf> {
        let key = format!(
            "{}:{}:{}",
            talent_id.unwrap_or(0),
            normalized(champion_name),
            normalized(talent_name)
        );
        if let Some(v) = self.talent_icons.read().unwrap().get(&key) {
            return v.clone();
        }
        let files = self.load_champion_files();
        let asset_name = if champion_name == "Seris" && talent_name == "Resuscitate" {
            "Seris Soul Collector"
        } else {
            &format!("{} {}", champion_name, talent_name)
        };
        let wanted = normalized(&format!("talent {}", asset_name));
        let result = files
            .iter()
            .filter(|f| {
                f.file_stem()
                    .map(|s| normalized(&s.to_string_lossy()) == wanted)
                    .unwrap_or(false)
            })
            .find(|f| f.extension().map(|e| e == "png").unwrap_or(false))
            .or_else(|| {
                files.iter().find(|f| {
                    f.file_stem()
                        .map(|s| normalized(&s.to_string_lossy()) == wanted)
                        .unwrap_or(false)
                })
            })
            .cloned();
        self.talent_icons
            .write()
            .unwrap()
            .insert(key, result.clone());
        result
    }

    pub fn map_image(&self, map_name: &str) -> Option<PathBuf> {
        let cleaned: String = map_name
            .split_whitespace()
            .filter(|w| !matches!(w.to_lowercase().as_str(), "ranked" | "live" | "wip"))
            .filter(|w| !w.starts_with('v') || !w[1..].chars().all(|c| c.is_ascii_digit()))
            .collect::<Vec<_>>()
            .join(" ")
            .trim()
            .to_string();
        let wanted = normalized(&cleaned);
        if let Some(v) = self.map_images.read().unwrap().get(&wanted) {
            return v.clone();
        }
        let files = self.load_map_files();
        let result = files
            .iter()
            .find(|f| {
                f.file_stem()
                    .map(|s| {
                        normalized(&s.to_string_lossy())
                            == normalized(&format!("ranked {}", wanted))
                    })
                    .unwrap_or(false)
            })
            .or_else(|| {
                files.iter().find(|f| {
                    let n = normalized(&f.to_string_lossy());
                    n.contains(&wanted) && n.contains("ranked")
                })
            })
            .or_else(|| {
                files
                    .iter()
                    .find(|f| normalized(&f.to_string_lossy()).contains(&wanted))
            })
            .cloned();
        self.map_images
            .write()
            .unwrap()
            .insert(wanted, result.clone());
        result
    }

    pub fn rank_icon(&self, tier: u32) -> Option<PathBuf> {
        if let Some(v) = self.rank_icons.read().unwrap().get(&tier) {
            return v.clone();
        }
        let files = self.load_rank_files();
        let result = if tier == 0 {
            files
                .iter()
                .find(|f| normalized(&f.to_string_lossy()).contains("rankiconqualifying"))
                .cloned()
        } else if tier >= 27 {
            files
                .iter()
                .find(|f| normalized(&f.to_string_lossy()).contains("rankicongrandmaster"))
                .cloned()
        } else if tier == 26 {
            files
                .iter()
                .find(|f| normalized(&f.to_string_lossy()).contains("rankiconmaster"))
                .cloned()
        } else {
            let groups = ["Bronze", "Silver", "Gold", "Platinum", "Diamond"];
            let group = (tier.saturating_sub(1) / 5).min(4) as usize;
            let division = 5 - ((tier.saturating_sub(1)) % 5);
            let wanted = normalized(&format!("rankicon {} {}", groups[group], division));
            files
                .iter()
                .find(|f| {
                    f.file_stem()
                        .map(|s| normalized(&s.to_string_lossy()) == wanted)
                        .unwrap_or(false)
                })
                .cloned()
        };
        self.rank_icons
            .write()
            .unwrap()
            .insert(tier, result.clone());
        result
    }

    pub fn icon(&self, name: &str, preferred_ext: Option<&str>) -> Option<PathBuf> {
        let key = format!("{}:{}", normalized(name), preferred_ext.unwrap_or(""));
        if let Some(v) = self.icons.read().unwrap().get(&key) {
            return v.clone();
        }
        let files = self.load_icon_files();
        let wanted = normalized(name);
        let preferred = preferred_ext.map(|e| e.to_lowercase());
        let result = preferred
            .as_ref()
            .and_then(|ext| {
                files.iter().find(|f| {
                    let name_ok = f
                        .file_stem()
                        .map(|s| normalized(&s.to_string_lossy()) == wanted)
                        .unwrap_or(false);
                    let ext_ok = f.extension().map(|e| e == ext.as_str()).unwrap_or(false);
                    name_ok && ext_ok
                })
            })
            .cloned()
            .or_else(|| {
                files
                    .iter()
                    .find(|f| {
                        f.file_stem()
                            .map(|s| normalized(&s.to_string_lossy()) == wanted)
                            .unwrap_or(false)
                    })
                    .cloned()
            });
        self.icons.write().unwrap().insert(key, result.clone());
        result
    }

    fn load_champion_files(&self) -> Vec<PathBuf> {
        self.load_files_with_lock(&self.champion_files, "champions", false)
    }
    fn load_map_files(&self) -> Vec<PathBuf> {
        self.load_files_with_lock(&self.map_files, "maps", false)
    }
    fn load_rank_files(&self) -> Vec<PathBuf> {
        self.load_files_with_lock(&self.rank_files, "rank-tiers", true)
    }
    fn load_icon_files(&self) -> Vec<PathBuf> {
        self.load_files_with_lock(&self.icon_files, "icons", false)
    }

    fn load_files_with_lock(
        &self,
        lock: &Arc<RwLock<Option<Vec<PathBuf>>>>,
        dir: &str,
        recursive: bool,
    ) -> Vec<PathBuf> {
        if let Some(files) = lock.read().unwrap().as_ref() {
            return files.clone();
        }
        drop(lock.read().unwrap());
        let files = self.load_files(dir, recursive);
        lock.write().unwrap().get_or_insert_with(|| files.clone());
        files
    }

    fn load_files(&self, directory: &str, recursive: bool) -> Vec<PathBuf> {
        let dir = self.root.join(directory);
        if !dir.exists() {
            return Vec::new();
        }
        match std::fs::read_dir(&dir) {
            Ok(entries) => entries
                .filter_map(|e| e.ok())
                .flat_map(|e| {
                    let p = e.path();
                    if p.is_file() {
                        if is_image_ext(&p) {
                            vec![p]
                        } else {
                            Vec::new()
                        }
                    } else if recursive {
                        self.load_nested_files(&p)
                    } else {
                        Vec::new()
                    }
                })
                .collect(),
            Err(_) => Vec::new(),
        }
    }

    fn load_nested_files(&self, directory: &Path) -> Vec<PathBuf> {
        let mut files = Vec::new();
        match std::fs::read_dir(directory) {
            Ok(entries) => {
                for entry in entries.filter_map(|e| e.ok()) {
                    let p = entry.path();
                    if p.is_file() {
                        if is_image_ext(&p) {
                            files.push(p);
                        }
                    } else {
                        files.extend(self.load_nested_files(&p));
                    }
                }
            }
            Err(_) => {}
        }
        files
    }
}
