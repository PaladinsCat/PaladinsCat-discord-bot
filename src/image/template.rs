//! HTML template data binding — injects JSON data into HTML templates.

use std::fs;
use std::sync::Arc;

#[derive(Debug, Clone)]
pub struct TemplateConfig {
    pub match_template_path: String,
    pub loadout_template_path: String,
    pub cheater_pattern_path: String,
}

impl TemplateConfig {
    pub fn dev_defaults() -> Self {
        Self {
            match_template_path: "dev/prototypes/match-result-scoreboard.html".into(),
            loadout_template_path: "dev/prototypes/loadout-card-layout.html".into(),
            cheater_pattern_path: "dev/prototypes/cheater-police-line.svg".into(),
        }
    }
}

#[derive(Clone)]
pub struct TemplateEngine {
    match_template: Arc<String>,
    loadout_template: Arc<String>,
    cheater_pattern_url: String,
}

impl TemplateEngine {
    pub fn load(config: &TemplateConfig) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let match_template = fs::read_to_string(&config.match_template_path)
            .map_err(|e| format!("Failed to load match template {}: {}", config.match_template_path, e))?;
        let loadout_template = fs::read_to_string(&config.loadout_template_path)
            .map_err(|e| format!("Failed to load loadout template {}: {}", config.loadout_template_path, e))?;
        let cheater_pattern_url: String = if fs::exists(&config.cheater_pattern_path).unwrap_or(false) {
            let svg = fs::read_to_string(&config.cheater_pattern_path).unwrap_or_default();
            format!("data:image/svg+xml,{}", url_encode(&svg))
        } else {
            String::new()
        };
        Ok(Self {
            match_template: Arc::new(match_template),
            loadout_template: Arc::new(loadout_template),
            cheater_pattern_url,
        })
    }

    pub fn extract_css(template: &str) -> String {
        if let Some(idx) = template.find("<style") {
            let rest = &template[idx..];
            if let Some(end) = rest.find("</style>") {
                let css = &rest[..end + "</style>".len()];
                let start = "<style".len();
                let end_len = "</style>".len();
                if css.len() > start + end_len {
                    return css[start..css.len() - end_len].to_string();
                }
            }
        }
        String::new()
    }

    pub fn match_document(&self, data: &serde_json::Value) -> String {
        let json_str = escape_html(&serde_json::to_string(data).unwrap_or_default());
        let tmpl = self.match_template.as_ref();
        if let Some(pos) = tmpl.find("</head>") {
            format!("{}<script>var __renderData={};</script>{}", &tmpl[..pos], json_str, &tmpl[pos..])
        } else {
            format!("<script>var __renderData={};</script>{}", json_str, tmpl)
        }
    }

    pub fn loadout_document(&self, data: &serde_json::Value) -> String {
        let json_str = escape_html(&serde_json::to_string(data).unwrap_or_default());
        let tmpl = self.loadout_template.as_ref();
        if let Some(pos) = tmpl.find("</head>") {
            format!("{}<script>var __renderData={};</script>{}", &tmpl[..pos], json_str, &tmpl[pos..])
        } else {
            format!("<script>var __renderData={};</script>{}", json_str, tmpl)
        }
    }

    pub fn cheater_pattern_url(&self) -> &str { &self.cheater_pattern_url }
}

pub fn escape_html(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
     .replace('"', "&quot;").replace('\'', "&#39;")
}

fn url_encode(s: &str) -> String {
    let mut r = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            'A'..='Z' | 'a'..='z' | '0'..='9' | '-' | '.' | '_' | '~' => r.push(c),
            ' ' => r.push_str("%20"),
            '\n' => r.push_str("%0A"),
            '\r' => r.push_str("%0D"),
            _ => for b in c.to_string().as_bytes() { r.push_str(&format!("%{:02X}", *b)); }
        }
    }
    r
}

pub fn asset_to_data_url(path: &str) -> Option<String> {
    let p = std::path::Path::new(path);
    if !p.exists() { return None; }
    let ext = p.extension()?.to_string_lossy();
    let mime = match ext.as_ref() {
        "png" => "image/png", "webp" => "image/webp", "jpg" | "jpeg" => "image/jpeg",
        "avif" => "image/avif", "svg" => "image/svg+xml", _ => return None,
    };
    let bytes = std::fs::read(p).ok()?;
    Some(format!("data:{};base64,{}", mime, encode_b64(&bytes)))
}

fn encode_b64(bytes: &[u8]) -> String {
    let t = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut r = String::with_capacity(bytes.len() / 3 * 4 + 4);
    let mut i = 0;
    while i + 2 < bytes.len() {
        let (b0, b1, b2) = (bytes[i] as u32, bytes[i+1] as u32, bytes[i+2] as u32);
        r.push(t[(((b0>>2)&63) as usize)] as char);
        r.push(t[(((b0<<4)|(b1>>4)) as u8 as usize)] as char);
        r.push(t[(((b1<<2)|(b2>>6)) as u8 as usize)] as char);
        r.push(t[((b2&63) as usize)] as char);
        i += 3;
    }
    let rem = bytes.len() - i;
    if rem == 1 { let b0=bytes[i] as u32; r.push(t[(((b0>>2)&63) as usize)] as char); r.push(t[((b0<<4) as u8 as usize)] as char); r.push('='); r.push('='); }
    else if rem == 2 { let (b0,b1)=(bytes[i] as u32,bytes[i+1] as u32); r.push(t[(((b0>>2)&63) as usize)] as char); r.push(t[(((b0<<4)|(b1>>4)) as u8 as usize)] as char); r.push(t[((b1<<2) as u8 as usize)] as char); r.push('='); }
    r
}
