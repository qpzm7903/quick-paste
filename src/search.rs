//! 智能检索引擎：支持多关键词 AND、前缀加权、标签、来源应用、中文拼音全拼/首字母及模糊子序列匹配。

use pinyin::ToPinyin;

use crate::content::ContentKind;

#[derive(Debug, Clone)]
pub struct SearchIndexEntry {
    pub text_lower: String,
    pub title_lower: String,
    pub tags_lower: Vec<String>,
    pub source_lower: String,
    pub kind_keywords: &'static str,
    pub pinyin_full: String,
    pub pinyin_initials: String,
}

pub fn build_entry(
    title: Option<&str>,
    text: &str,
    tags: &[String],
    source_app: Option<&str>,
    kind: ContentKind,
) -> SearchIndexEntry {
    let title_lower = title.unwrap_or("").trim().to_lowercase();
    let text_lower = text.to_lowercase();
    let tags_lower: Vec<String> = tags.iter().map(|t| t.to_lowercase()).collect();
    let source_lower = source_app.unwrap_or("").trim().to_lowercase();

    let (pinyin_full, pinyin_initials) = build_pinyin_fields(title.unwrap_or(""), text, tags);

    SearchIndexEntry {
        text_lower,
        title_lower,
        tags_lower,
        source_lower,
        kind_keywords: kind.search_keywords(),
        pinyin_full,
        pinyin_initials,
    }
}

fn build_pinyin_fields(title: &str, text: &str, tags: &[String]) -> (String, String) {
    let mut full = String::with_capacity(128);
    let mut initials = String::with_capacity(64);
    let mut last_was_space = true;

    let push_space = |full: &mut String, initials: &mut String, last_space: &mut bool| {
        if !*last_space {
            full.push(' ');
            initials.push(' ');
            *last_space = true;
        }
    };

    let feed =
        |s: &str, limit: usize, full: &mut String, initials: &mut String, last_space: &mut bool| {
            for ch in s.chars().take(limit) {
                if let Some(py) = ch.to_pinyin() {
                    let plain = py.plain();
                    full.push_str(plain);
                    if let Some(first) = plain.chars().next() {
                        initials.push(first);
                    }
                    *last_space = false;
                } else if ch.is_ascii_alphanumeric() {
                    let lower = ch.to_ascii_lowercase();
                    full.push(lower);
                    initials.push(lower);
                    *last_space = false;
                } else {
                    push_space(full, initials, last_space);
                }
            }
            push_space(full, initials, last_space);
        };

    if !title.is_empty() {
        feed(title, 128, &mut full, &mut initials, &mut last_was_space);
    }
    feed(text, 512, &mut full, &mut initials, &mut last_was_space);
    for tag in tags {
        feed(tag, 64, &mut full, &mut initials, &mut last_was_space);
    }

    (full, initials)
}

/// 按多 token AND 语义计算匹配得分；未命中返回 `None`。
pub fn score_entry(entry: &SearchIndexEntry, query: &str) -> Option<i64> {
    let trimmed = query.trim().to_lowercase();
    if trimmed.is_empty() {
        return Some(0);
    }

    let mut total_score: i64 = 0;
    for token in trimmed.split_whitespace() {
        let token_score = score_token(entry, token)?;
        total_score += token_score;
    }
    Some(total_score)
}

fn score_token(entry: &SearchIndexEntry, token: &str) -> Option<i64> {
    let mut best: Option<i64> = None;
    let mut consider = |score: i64| {
        best = Some(best.map_or(score, |current| current.max(score)));
    };

    if !entry.title_lower.is_empty() {
        if entry.title_lower == token {
            consider(1100);
        } else if entry.title_lower.starts_with(token) {
            consider(1020);
        } else if entry.title_lower.contains(token) {
            consider(800);
        }
    }

    if entry.text_lower.starts_with(token) {
        consider(1000);
    } else if entry.text_lower.contains(token) {
        consider(700);
    }

    for tag in &entry.tags_lower {
        if tag == token {
            consider(920);
        } else if tag.contains(token) {
            consider(850);
        }
    }

    if !entry.source_lower.is_empty() && entry.source_lower.contains(token) {
        consider(520);
    }
    if entry.kind_keywords.contains(token) {
        consider(480);
    }

    if token.is_ascii() {
        if entry.pinyin_initials.contains(token) {
            consider(450);
        }
        if entry.pinyin_full.contains(token) {
            consider(400);
        }
        if token.chars().count() >= 2 {
            if let Some(fuzzy) = fuzzy_subsequence_score(&entry.text_lower, token) {
                consider(fuzzy);
            }
            if !entry.title_lower.is_empty()
                && let Some(fuzzy) = fuzzy_subsequence_score(&entry.title_lower, token)
            {
                consider(fuzzy + 40);
            }
        }
    }

    best
}

fn fuzzy_subsequence_score(haystack: &str, needle: &str) -> Option<i64> {
    let mut needle_chars = needle.chars();
    let mut current = needle_chars.next()?;
    let mut first_idx = None;

    for (idx, ch) in haystack.chars().take(256).enumerate() {
        if ch == current {
            if first_idx.is_none() {
                first_idx = Some(idx);
            }
            match needle_chars.next() {
                Some(next) => current = next,
                None => {
                    let span = (idx - first_idx.unwrap_or(0)) as i64;
                    let needle_len = needle.chars().count() as i64;
                    let gap = (span + 1 - needle_len).max(0);
                    return Some((260 - gap * 6).max(80));
                }
            }
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_exact_prefix_tags_and_source_app() {
        let entry = build_entry(
            Some("工作邮箱"),
            "me@company.com",
            &["办公".to_owned(), "Email".to_owned()],
            Some("Google Chrome"),
            ContentKind::Text,
        );

        assert!(score_entry(&entry, "").is_some());
        let prefix = score_entry(&entry, "me@").expect("prefix");
        let substr = score_entry(&entry, "company").expect("substr");
        assert!(prefix > substr);
        assert!(score_entry(&entry, "email").is_some());
        assert!(score_entry(&entry, "chrome").is_some());
        assert!(score_entry(&entry, "chrome company").is_some());
        assert!(score_entry(&entry, "chrome nonexistent").is_none());
    }

    #[test]
    fn matches_chinese_pinyin_full_and_initials() {
        let entry = build_entry(
            None,
            "微信公众号回复模板",
            &["运营".to_owned()],
            None,
            ContentKind::Text,
        );

        assert!(score_entry(&entry, "wxgzh").is_some());
        assert!(score_entry(&entry, "weixin").is_some());
        assert!(score_entry(&entry, "yy").is_some());
    }

    #[test]
    fn matches_fuzzy_subsequence() {
        let entry = build_entry(
            None,
            "git push origin main",
            &[],
            Some("Terminal"),
            ContentKind::Code,
        );
        assert!(score_entry(&entry, "gpo").is_some());
        assert!(score_entry(&entry, "code").is_some());
    }
}
