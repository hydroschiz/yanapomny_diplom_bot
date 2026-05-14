//! Утилиты форматирования текста для транспортного слоя.
//!
//! VK не поддерживает HTML-разметку в сообщениях, в отличие от Telegram.
//! Этот модуль предоставляет функции для конвертации HTML-форматированного
//! текста в plain text, сохраняя читаемость.

/// Удаляет HTML-теги из текста, сохраняя содержимое.
///
/// Поддерживаемые преобразования:
/// - `<b>text</b>` → `text` (жирный → plain)
/// - `<i>text</i>` → `text` (курсив → plain)
/// - `<a href="url">text</a>` → `text (url)` (ссылка → text + url в скобках)
/// - `&amp;` → `&`
/// - `&lt;` → `<`
/// - `&gt;` → `>`
/// - Прочие теги удаляются
///
/// # Примеры
///
/// ```
/// use yanapomnyu_bot::transport::text_format::strip_html;
///
/// assert_eq!(strip_html("<b>Привет</b>, мир!"), "Привет, мир!");
/// assert_eq!(
///     strip_html(r#"<a href="https://example.com">Ссылка</a>"#),
///     "Ссылка (https://example.com)"
/// );
/// assert_eq!(strip_html("Tom &amp; Jerry"), "Tom & Jerry");
/// ```
pub fn strip_html(html: &str) -> String {
    use regex::Regex;

    // 1. Обрабатываем <a href="url">text</a> → text (url)
    //    (если текст == url, выводим только текст)
    let re_link = Regex::new(r#"<a\s+[^>]*href\s*=\s*["']([^"']*)["'][^>]*>(.*?)</a>"#).unwrap();
    let s = re_link.replace_all(html, |caps: &regex::Captures| {
        let url = &caps[1];
        let text = &caps[2];
        if text.trim() == url.trim() {
            text.to_string()
        } else {
            format!("{} ({})", text, url)
        }
    });

    // 2. Удаляем все оставшиеся HTML-теги (<b>, </b>, <i>, </i>, и т.д.)
    let re_tags = Regex::new(r"<[^>]*>").unwrap();
    let s = re_tags.replace_all(&s, "");

    // 3. Декодируем HTML entities
    s.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&nbsp;", " ")
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_strip_bold() {
        assert_eq!(strip_html("<b>Привет</b>"), "Привет");
    }

    #[test]
    fn test_strip_italic() {
        assert_eq!(strip_html("<i>курсив</i>"), "курсив");
    }

    #[test]
    fn test_strip_bold_italic_mixed() {
        assert_eq!(
            strip_html("<b>Статус:</b> <i>активна</i>"),
            "Статус: активна"
        );
    }

    #[test]
    fn test_strip_link() {
        assert_eq!(
            strip_html(r#"<a href="https://example.com">Ссылка</a>"#),
            "Ссылка (https://example.com)"
        );
    }

    #[test]
    fn test_strip_link_url_equals_text() {
        assert_eq!(
            strip_html(r#"<a href="https://example.com">https://example.com</a>"#),
            "https://example.com"
        );
    }

    #[test]
    fn test_strip_entities() {
        assert_eq!(strip_html("Tom &amp; Jerry"), "Tom & Jerry");
        assert_eq!(strip_html("1 &lt; 2 &gt; 0"), "1 < 2 > 0");
        assert_eq!(strip_html("&quot;hello&quot;"), "\"hello\"");
    }

    #[test]
    fn test_plain_text_passthrough() {
        assert_eq!(
            strip_html("Обычный текст без тегов"),
            "Обычный текст без тегов"
        );
    }

    #[test]
    fn test_empty_string() {
        assert_eq!(strip_html(""), "");
    }

    #[test]
    fn test_complex_message() {
        let html = "👛 <b>Выберите срок</b>\n\n📧 <b>Статус:</b> активна ✅\n\n<i>Совет:</i> <b>выбирайте более длительную подписку</b>.";
        let expected = "👛 Выберите срок\n\n📧 Статус: активна ✅\n\nСовет: выбирайте более длительную подписку.";
        assert_eq!(strip_html(html), expected);
    }

    #[test]
    fn test_message_with_link() {
        let html =
            r#"Оплачивая, вы <a href="https://telegra.ph/agreement">принимаете условия</a>."#;
        let expected = "Оплачивая, вы принимаете условия (https://telegra.ph/agreement).";
        assert_eq!(strip_html(html), expected);
    }
}
