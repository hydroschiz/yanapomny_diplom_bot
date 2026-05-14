pub fn strip_html(html: &str) -> String {
    use regex::Regex;

    let re_link = Regex::new(r#"<a\s+[^>]*href\s*=\s*[\"']([^\"']*)[\"'][^>]*>(.*?)</a>"#).unwrap();
    let value = re_link.replace_all(html, |captures: &regex::Captures| {
        let url = &captures[1];
        let text = &captures[2];
        if text.trim() == url.trim() {
            text.to_string()
        } else {
            format!("{} ({})", text, url)
        }
    });

    let re_tags = Regex::new(r"<[^>]*>").unwrap();
    let value = re_tags.replace_all(&value, "");

    value
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&nbsp;", " ")
}
