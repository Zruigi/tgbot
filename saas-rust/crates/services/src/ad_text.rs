// 广告文本归一化与哈希 — 消息存储与广告聚类共用的稳定归一化
// 对应 Python: services/ad_text.py
//
// 重要：`ad_text_hash` 的 MD5(归一化文本) 必须与 Python 位级一致
// （移植规格 PORTING-13-COMMITS.md #4），否则租户广告指纹库迁移后失效。

use md5::{Digest, Md5};
use regex::Regex;
use std::sync::LazyLock;

/// 对应 Python `_URL_RE = re.compile(r"https?://\S+|t\.me/\S+|@\w{3,}", re.I)`
static URL_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)https?://\S+|t\.me/\S+|@\w{3,}").unwrap());

/// 对应 Python `_SPACE_RE = re.compile(r"\s+")`
static SPACE_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\s+").unwrap());

/// 归一化文本用于精确重复分组（不隐藏意图）。
///
/// 与 Python `normalize_ad_text` 保持一致：
/// strip → lowercase → URL/t.me/@handle 替换为 "[link]" → 空白折叠 → strip
pub fn normalize_ad_text(text: &str) -> String {
    let value = text.trim().to_lowercase();
    let value = URL_RE.replace_all(&value, "[link]");
    SPACE_RE.replace_all(&value, " ").trim().to_string()
}

/// 归一化文本的 MD5 十六进制哈希（与 Python `ad_text_hash` 位级一致）。
pub fn ad_text_hash(text: &str) -> String {
    let mut hasher = Md5::new();
    hasher.update(normalize_ad_text(text).as_bytes());
    hex::encode(hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_urls_and_whitespace() {
        assert_eq!(
            normalize_ad_text("  Hello   https://example.com/x  World "),
            "hello [link] world"
        );
        assert_eq!(
            normalize_ad_text("加我 @Spammer123 详谈"),
            "加我 [link] 详谈"
        );
        assert_eq!(normalize_ad_text("t.me/joinchat/abc 进群"), "[link] 进群");
    }

    #[test]
    fn normalization_is_case_insensitive_and_stable() {
        assert_eq!(normalize_ad_text("ABC  def"), normalize_ad_text("abc def"));
        assert_eq!(normalize_ad_text(""), "");
        assert_eq!(normalize_ad_text("   \n\t "), "");
    }

    #[test]
    fn hash_matches_python_md5_reference() {
        // Python: hashlib.md5("".encode()).hexdigest()
        assert_eq!(ad_text_hash(""), "d41d8cd98f00b204e9800998ecf8427e");
        // Python: ad_text_hash("Hello   World") == md5("hello world")
        assert_eq!(
            ad_text_hash("Hello   World"),
            "5eb63bbbe01eeed093cb22bb8f5acdc3"
        );
        // Python: ad_text_hash("看 https://t.me/x 详情") == md5("看 [link] 详情")
        // 中文 UTF-8 编码路径也必须一致
        assert_eq!(
            ad_text_hash("看 https://t.me/x 详情"),
            ad_text_hash("看 t.me/y 详情")
        );
    }

    #[test]
    fn same_ad_different_links_cluster_together() {
        let a = ad_text_hash("低价出料 联系 @seller_a");
        let b = ad_text_hash("低价出料 联系 @seller_b");
        assert_eq!(a, b);
    }
}
