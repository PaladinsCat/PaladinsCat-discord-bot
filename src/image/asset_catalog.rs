//! Game asset lookup — mirrors TS `asset-catalog.ts`.

use std::collections::{HashMap, HashSet};
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
    card_reference: Arc<RwLock<Option<HashMap<u32, LoadoutCardAsset>>>>,
    frame_reference: Arc<RwLock<Option<HashMap<u32, LoadoutFrameAsset>>>>,
}

#[derive(Clone)]
pub struct LoadoutCardAsset {
    pub name: String,
    pub description: String,
    pub short_description: String,
    pub icon_path: Option<PathBuf>,
}

#[derive(Clone)]
pub struct LoadoutFrameAsset {
    pub rarity: String,
    pub icon_path: PathBuf,
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
            card_reference: Arc::new(RwLock::new(None)),
            frame_reference: Arc::new(RwLock::new(None)),
        }
    }

    pub fn loadout_card(&self, card_id: u32) -> Option<LoadoutCardAsset> {
        if self.card_reference.read().unwrap().is_none() {
            let loaded = self.load_card_reference();
            *self.card_reference.write().unwrap() = Some(loaded);
        }
        self.card_reference
            .read()
            .unwrap()
            .as_ref()?
            .get(&card_id)
            .cloned()
    }

    pub fn loadout_frame(&self, level: u32) -> Option<LoadoutFrameAsset> {
        if self.frame_reference.read().unwrap().is_none() {
            let loaded = self.load_frame_reference();
            *self.frame_reference.write().unwrap() = Some(loaded);
        }
        self.frame_reference
            .read()
            .unwrap()
            .as_ref()?
            .get(&level.clamp(1, 5))
            .cloned()
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

    fn public_image_path(&self, url: &str) -> Option<PathBuf> {
        self.resolve_public_image_path(url)
            .filter(|path| path.exists())
    }

    fn resolve_public_image_path(&self, url: &str) -> Option<PathBuf> {
        Some(self.root.join(url.strip_prefix("/images/")?))
    }

    fn load_card_reference(&self) -> HashMap<u32, LoadoutCardAsset> {
        let path = self
            .root
            .parent()
            .unwrap_or(&self.root)
            .join("data/paladins-card-reference.json");
        let Ok(bytes) = std::fs::read(path) else {
            return HashMap::new();
        };
        let Ok(rows) = serde_json::from_slice::<Vec<serde_json::Value>>(&bytes) else {
            return HashMap::new();
        };
        let descriptions = self.load_champion_card_descriptions();
        rows.into_iter()
            .filter_map(|row| {
                let id = row.get("id")?.as_u64()? as u32;
                if id == 0 {
                    return None;
                }
                let name = row
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("Unknown Card")
                    .to_string();
                let canonical = row
                    .get("iconUrl")
                    .and_then(|v| v.as_str())
                    .and_then(|url| self.resolve_public_image_path(url));
                let png = canonical
                    .as_ref()
                    .map(|path| path.with_extension("png"))
                    .filter(|path| path.exists());
                let canonical = canonical.filter(|path| path.exists());
                Some((
                    id,
                    LoadoutCardAsset {
                        description: descriptions
                            .get(&normalized(&name))
                            .cloned()
                            .or_else(|| {
                                row.get("description")
                                    .and_then(|v| v.as_str())
                                    .map(str::to_owned)
                            })
                            .unwrap_or_default(),
                        short_description: row
                            .get("shortDescription")
                            .and_then(|v| v.as_str())
                            .unwrap_or_default()
                            .to_string(),
                        icon_path: png.or(canonical),
                        name,
                    },
                ))
            })
            .collect()
    }

    fn load_champion_card_descriptions(&self) -> HashMap<String, String> {
        let path = self
            .root
            .parent()
            .unwrap_or(&self.root)
            .join("data/champion-data.json");
        let Ok(bytes) = std::fs::read(path) else {
            return HashMap::new();
        };
        let Ok(champions) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
            return HashMap::new();
        };
        let mut descriptions = HashMap::new();
        let mut ambiguous = HashSet::new();
        for champion in champions
            .as_object()
            .into_iter()
            .flat_map(|value| value.values())
        {
            for card in champion
                .get("loadouts")
                .and_then(|v| v.as_array())
                .into_iter()
                .flatten()
            {
                let name = card
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default();
                let description = card
                    .get("description")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .trim();
                let key = normalized(name);
                if key.is_empty() || description.is_empty() || ambiguous.contains(&key) {
                    continue;
                }
                if descriptions
                    .get(&key)
                    .is_some_and(|existing| existing != description)
                {
                    descriptions.remove(&key);
                    ambiguous.insert(key);
                } else {
                    descriptions.insert(key, description.to_string());
                }
            }
        }
        descriptions
    }

    fn load_frame_reference(&self) -> HashMap<u32, LoadoutFrameAsset> {
        let path = self
            .root
            .parent()
            .unwrap_or(&self.root)
            .join("data/paladins-loadout-frame-reference.json");
        let Ok(bytes) = std::fs::read(path) else {
            return HashMap::new();
        };
        let Ok(rows) = serde_json::from_slice::<Vec<serde_json::Value>>(&bytes) else {
            return HashMap::new();
        };
        rows.into_iter()
            .filter_map(|row| {
                let level = row.get("level")?.as_u64()? as u32;
                if !(1..=5).contains(&level) {
                    return None;
                }
                let path = row
                    .get("pngUrl")
                    .and_then(|v| v.as_str())
                    .and_then(|url| self.public_image_path(url))
                    .or_else(|| {
                        row.get("iconUrl")
                            .and_then(|v| v.as_str())
                            .and_then(|url| self.public_image_path(url))
                    })?;
                Some((
                    level,
                    LoadoutFrameAsset {
                        rarity: row
                            .get("rarity")
                            .and_then(|v| v.as_str())
                            .unwrap_or("Unknown")
                            .to_string(),
                        icon_path: path,
                    },
                ))
            })
            .collect()
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

#[cfg(test)]
mod tests {
    use super::AssetCatalog;

    #[test]
    fn resolves_packaged_png_when_referenced_avif_is_absent() {
        let fixture =
            std::env::temp_dir().join(format!("paladinscat-card-catalog-{}", uuid::Uuid::new_v4()));
        let images = fixture.join("images");
        let cards = images.join("cards");
        let data = fixture.join("data");
        std::fs::create_dir_all(&cards).unwrap();
        std::fs::create_dir_all(&data).unwrap();
        std::fs::write(cards.join("Card_Test.png"), b"png-only-fixture").unwrap();
        std::fs::write(
            data.join("paladins-card-reference.json"),
            br#"[{"id":42,"name":"Test","iconUrl":"/images/cards/Card_Test.avif"}]"#,
        )
        .unwrap();

        let asset = AssetCatalog::new(&images).loadout_card(42).unwrap();
        assert_eq!(asset.icon_path, Some(cards.join("Card_Test.png")));

        std::fs::remove_dir_all(fixture).unwrap();
    }
}
