//! 剪贴板历史本地持久化：支持文本与图片去重、容量淘汰、置顶保护与磁盘图片管理。

use std::{
    collections::HashSet,
    fs,
    io::{Cursor, Write as _},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use atomic_write_file::AtomicWriteFile;
use image::{ImageFormat, RgbaImage, imageops::FilterType};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::content::{self, ContentKind};

const THUMB_MAX_WIDTH: u32 = 320;
const THUMB_MAX_HEIGHT: u32 = 180;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HistoryItem {
    pub id: String,
    pub content_hash: String,
    pub kind: ContentKind,
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image_file: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thumb_file: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image_width: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image_height: Option<u32>,
    #[serde(default)]
    pub byte_size: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_app: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_app_id: Option<String>,
    pub copied_at: u64,
    #[serde(default)]
    pub last_used_at: u64,
    #[serde(default)]
    pub usage_count: u64,
    #[serde(default)]
    pub is_pinned: bool,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct HistoryDocument {
    items: Vec<HistoryItem>,
}

pub struct HistoryStore {
    history_path: PathBuf,
    images_dir: PathBuf,
    items: Vec<HistoryItem>,
}

impl HistoryStore {
    pub fn load(data_dir: &Path) -> Self {
        let history_path = data_dir.join("history.json");
        let images_dir = data_dir.join("images");
        let mut items = if history_path.exists() {
            fs::read_to_string(&history_path)
                .ok()
                .and_then(|raw| serde_json::from_str::<HistoryDocument>(&raw).ok())
                .map(|doc| doc.items)
                .unwrap_or_default()
        } else {
            Vec::new()
        };
        sort_for_display(&mut items);
        Self {
            history_path,
            images_dir,
            items,
        }
    }

    pub fn items(&self) -> &[HistoryItem] {
        &self.items
    }

    pub fn image_path(&self, file_name: &str) -> PathBuf {
        self.images_dir.join(file_name)
    }

    pub fn save(&self) -> Result<(), String> {
        if let Some(parent) = self.history_path.parent() {
            fs::create_dir_all(parent).map_err(|e| format!("创建历史目录失败：{e}"))?;
        }
        let doc = HistoryDocument {
            items: self.items.clone(),
        };
        let json =
            serde_json::to_string_pretty(&doc).map_err(|e| format!("序列化剪贴板历史失败：{e}"))?;
        let mut file = AtomicWriteFile::open(&self.history_path)
            .map_err(|e| format!("打开历史文件失败：{e}"))?;
        file.write_all(json.as_bytes())
            .map_err(|e| format!("写入历史文件失败：{e}"))?;
        file.as_file()
            .sync_all()
            .map_err(|e| format!("同步历史文件失败：{e}"))?;
        file.commit()
            .map_err(|e| format!("提交历史文件失败：{e}"))?;
        Ok(())
    }

    /// 记录文本剪贴板条目，返回是否发生了列表变更。
    pub fn record_text(
        &mut self,
        text: String,
        source_app: Option<String>,
        source_app_id: Option<String>,
        max_items: usize,
    ) -> Result<bool, String> {
        self.record_text_at(text, source_app, source_app_id, max_items, now_unix_secs())
    }

    pub fn record_text_at(
        &mut self,
        text: String,
        source_app: Option<String>,
        source_app_id: Option<String>,
        max_items: usize,
        now_secs: u64,
    ) -> Result<bool, String> {
        if text.trim().is_empty() {
            return Ok(false);
        }

        let content_hash = sha256_tagged(b"text:", text.as_bytes());
        if let Some(existing) = self
            .items
            .iter_mut()
            .find(|item| item.content_hash == content_hash)
        {
            existing.copied_at = now_secs;
            if source_app.is_some() {
                existing.source_app = source_app;
            }
            if source_app_id.is_some() {
                existing.source_app_id = source_app_id;
            }
            sort_for_display(&mut self.items);
            self.save()?;
            return Ok(true);
        }

        let id = content_hash[..16].to_owned();
        let kind = content::classify_text(&text);
        let byte_size = text.len() as u64;
        self.items.push(HistoryItem {
            id,
            content_hash,
            kind,
            text,
            image_file: None,
            thumb_file: None,
            image_width: None,
            image_height: None,
            byte_size,
            source_app,
            source_app_id,
            copied_at: now_secs,
            last_used_at: 0,
            usage_count: 0,
            is_pinned: false,
        });

        self.enforce_capacity(max_items);
        sort_for_display(&mut self.items);
        self.save()?;
        Ok(true)
    }

    /// 记录 RGBA 图片剪贴板条目，返回是否成功写入新条目或更新去重条目。
    #[allow(clippy::too_many_arguments)]
    pub fn record_image(
        &mut self,
        rgba: &[u8],
        width: u32,
        height: u32,
        source_app: Option<String>,
        source_app_id: Option<String>,
        max_items: usize,
        max_image_mb: usize,
    ) -> Result<bool, String> {
        self.record_image_at(
            rgba,
            width,
            height,
            source_app,
            source_app_id,
            max_items,
            max_image_mb,
            now_unix_secs(),
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn record_image_at(
        &mut self,
        rgba: &[u8],
        width: u32,
        height: u32,
        source_app: Option<String>,
        source_app_id: Option<String>,
        max_items: usize,
        max_image_mb: usize,
        now_secs: u64,
    ) -> Result<bool, String> {
        if width == 0 || height == 0 {
            return Ok(false);
        }
        let expected_len = (width as usize)
            .checked_mul(height as usize)
            .and_then(|pixels| pixels.checked_mul(4))
            .ok_or_else(|| "图片尺寸过大".to_owned())?;
        if rgba.len() != expected_len {
            return Err("图片像素缓冲区长度不匹配".to_owned());
        }

        let raw_limit = max_image_mb.saturating_mul(8 * 1024 * 1024);
        if rgba.len() > raw_limit {
            return Ok(false);
        }

        let tag = format!("rgba:{width}x{height}:");
        let content_hash = sha256_tagged(tag.as_bytes(), rgba);
        if let Some(existing) = self
            .items
            .iter_mut()
            .find(|item| item.content_hash == content_hash)
        {
            existing.copied_at = now_secs;
            if source_app.is_some() {
                existing.source_app = source_app;
            }
            if source_app_id.is_some() {
                existing.source_app_id = source_app_id;
            }
            sort_for_display(&mut self.items);
            self.save()?;
            return Ok(true);
        }

        let image_buf = RgbaImage::from_raw(width, height, rgba.to_vec())
            .ok_or_else(|| "构造图片缓冲区失败".to_owned())?;
        let png_bytes = encode_png(&image_buf)?;
        let png_limit = max_image_mb.saturating_mul(1024 * 1024);
        if png_bytes.len() > png_limit {
            return Ok(false);
        }

        let thumb_bytes = if width > THUMB_MAX_WIDTH || height > THUMB_MAX_HEIGHT {
            let dyn_img = image::DynamicImage::ImageRgba8(image_buf);
            let resized = dyn_img.resize(THUMB_MAX_WIDTH, THUMB_MAX_HEIGHT, FilterType::Triangle);
            encode_png(&resized.to_rgba8())?
        } else {
            png_bytes.clone()
        };

        fs::create_dir_all(&self.images_dir).map_err(|e| format!("创建图片缓存目录失败：{e}"))?;
        let id = content_hash[..16].to_owned();
        let image_file = format!("{id}.png");
        let thumb_file = format!("{id}_thumb.png");

        write_atomic_bytes(&self.images_dir.join(&image_file), &png_bytes)?;
        write_atomic_bytes(&self.images_dir.join(&thumb_file), &thumb_bytes)?;

        let byte_size = png_bytes.len() as u64;
        self.items.push(HistoryItem {
            id,
            content_hash,
            kind: ContentKind::Image,
            text: format!("图片 {width}×{height}"),
            image_file: Some(image_file),
            thumb_file: Some(thumb_file),
            image_width: Some(width),
            image_height: Some(height),
            byte_size,
            source_app,
            source_app_id,
            copied_at: now_secs,
            last_used_at: 0,
            usage_count: 0,
            is_pinned: false,
        });

        self.enforce_capacity(max_items);
        sort_for_display(&mut self.items);
        self.save()?;
        Ok(true)
    }

    pub fn toggle_pin(&mut self, id: &str) -> Result<Option<bool>, String> {
        let Some(item) = self.items.iter_mut().find(|item| item.id == id) else {
            return Ok(None);
        };
        item.is_pinned = !item.is_pinned;
        let pinned = item.is_pinned;
        sort_for_display(&mut self.items);
        self.save()?;
        Ok(Some(pinned))
    }

    pub fn mark_used(&mut self, id: &str) -> Result<bool, String> {
        let now = now_unix_secs();
        let Some(item) = self.items.iter_mut().find(|item| item.id == id) else {
            return Ok(false);
        };
        item.usage_count = item.usage_count.saturating_add(1);
        item.last_used_at = now;
        sort_for_display(&mut self.items);
        self.save()?;
        Ok(true)
    }

    pub fn delete(&mut self, id: &str) -> Result<bool, String> {
        let Some(pos) = self.items.iter().position(|item| item.id == id) else {
            return Ok(false);
        };
        let removed = self.items.remove(pos);
        self.remove_unreferenced_images(&[removed]);
        self.save()?;
        Ok(true)
    }

    pub fn clear_unpinned(&mut self) -> Result<usize, String> {
        let mut removed = Vec::new();
        let mut kept = Vec::with_capacity(self.items.len());
        for item in self.items.drain(..) {
            if item.is_pinned {
                kept.push(item);
            } else {
                removed.push(item);
            }
        }
        let count = removed.len();
        self.items = kept;
        if count > 0 {
            self.remove_unreferenced_images(&removed);
            self.save()?;
        }
        Ok(count)
    }

    fn enforce_capacity(&mut self, max_items: usize) {
        if self.items.len() <= max_items {
            return;
        }
        sort_for_display(&mut self.items);
        let mut removed = Vec::new();
        while self.items.len() > max_items {
            // 由于已排序（置顶在前、最新在前），从尾部寻找最旧的未置顶记录淘汰。
            let Some(evict_idx) = self.items.iter().rposition(|item| !item.is_pinned) else {
                break;
            };
            removed.push(self.items.remove(evict_idx));
        }
        if !removed.is_empty() {
            self.remove_unreferenced_images(&removed);
        }
    }

    fn remove_unreferenced_images(&self, removed: &[HistoryItem]) {
        let referenced: HashSet<&str> = self
            .items
            .iter()
            .flat_map(|item| {
                item.image_file
                    .as_deref()
                    .into_iter()
                    .chain(item.thumb_file.as_deref())
            })
            .collect();

        for item in removed {
            for file in item
                .image_file
                .as_deref()
                .into_iter()
                .chain(item.thumb_file.as_deref())
            {
                if !referenced.contains(file) {
                    let _ = fs::remove_file(self.images_dir.join(file));
                }
            }
        }
    }
}

pub fn sort_for_display(items: &mut [HistoryItem]) {
    items.sort_by_key(|item| {
        (
            !item.is_pinned,
            std::cmp::Reverse(item.copied_at),
            std::cmp::Reverse(item.last_used_at),
        )
    });
}

pub fn sha256_tagged(prefix: &[u8], payload: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(prefix);
    hasher.update(payload);
    let digest = hasher.finalize();
    let mut out = String::with_capacity(digest.len() * 2);
    for byte in digest {
        use std::fmt::Write as _;
        let _ = write!(out, "{byte:02x}");
    }
    out
}

pub fn now_unix_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

fn encode_png(image_buf: &RgbaImage) -> Result<Vec<u8>, String> {
    let mut cursor = Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba8(image_buf.clone())
        .write_to(&mut cursor, ImageFormat::Png)
        .map_err(|e| format!("编码 PNG 图片失败：{e}"))?;
    Ok(cursor.into_inner())
}

fn write_atomic_bytes(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let mut file = AtomicWriteFile::open(path).map_err(|e| format!("保存图片失败：{e}"))?;
    file.write_all(bytes)
        .map_err(|e| format!("写入图片失败：{e}"))?;
    file.commit().map_err(|e| format!("提交图片失败：{e}"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deduplicates_text_preserving_pin_and_usage() {
        let dir = tempfile::tempdir().expect("temp dir");
        let mut store = HistoryStore::load(dir.path());

        assert!(
            store
                .record_text_at("hello".to_owned(), Some("Notes".to_owned()), None, 50, 100)
                .expect("record")
        );
        let id = store.items()[0].id.clone();
        store.toggle_pin(&id).expect("pin");
        store.mark_used(&id).expect("used");

        assert!(
            store
                .record_text_at(
                    "hello".to_owned(),
                    Some("Terminal".to_owned()),
                    None,
                    50,
                    200
                )
                .expect("dedup")
        );
        assert_eq!(store.items().len(), 1);
        assert!(store.items()[0].is_pinned);
        assert_eq!(store.items()[0].usage_count, 1);
        assert_eq!(store.items()[0].copied_at, 200);
        assert_eq!(store.items()[0].source_app.as_deref(), Some("Terminal"));
    }

    #[test]
    fn evicts_oldest_unpinned_items_only() {
        let dir = tempfile::tempdir().expect("temp dir");
        let mut store = HistoryStore::load(dir.path());

        store
            .record_text_at("pinned-old".to_owned(), None, None, 2, 10)
            .expect("1");
        let pinned_id = store.items()[0].id.clone();
        store.toggle_pin(&pinned_id).expect("pin");

        store
            .record_text_at("unpinned-old".to_owned(), None, None, 2, 20)
            .expect("2");
        store
            .record_text_at("unpinned-new".to_owned(), None, None, 2, 30)
            .expect("3");

        let texts: Vec<&str> = store.items().iter().map(|i| i.text.as_str()).collect();
        assert_eq!(texts, ["pinned-old", "unpinned-new"]);
    }

    #[test]
    fn saves_and_cleans_up_image_files() {
        let dir = tempfile::tempdir().expect("temp dir");
        let mut store = HistoryStore::load(dir.path());

        let rgba = vec![255u8; 400 * 200 * 4];
        assert!(
            store
                .record_image_at(
                    &rgba,
                    400,
                    200,
                    Some("Preview".to_owned()),
                    None,
                    10,
                    10,
                    50
                )
                .expect("record img")
        );
        assert_eq!(store.items().len(), 1);

        let item = &store.items()[0];
        let img_path = store.image_path(item.image_file.as_ref().unwrap());
        let thumb_path = store.image_path(item.thumb_file.as_ref().unwrap());
        assert!(img_path.exists());
        assert!(thumb_path.exists());

        let reloaded = HistoryStore::load(dir.path());
        assert_eq!(reloaded.items().len(), 1);
        assert_eq!(reloaded.items()[0].kind, ContentKind::Image);

        let id = item.id.clone();
        store.delete(&id).expect("delete");
        assert!(!img_path.exists());
        assert!(!thumb_path.exists());
    }
}
