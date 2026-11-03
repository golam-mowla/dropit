use std::path::{Path, PathBuf};

/// Sanitizes a filename supplied by a remote peer.
/// Ensures that the resulting name is safe to write to the local destination directory.
pub fn sanitize_filename(raw_name: &str) -> String {
    // 1. Strip leading and trailing whitespace
    let mut name = raw_name.trim();

    // 2. Extract only the last path segment (handles both / and \)
    if let Some(pos) = name.rfind(['/', '\\']) {
        name = &name[pos + 1..];
    }

    // 3. Strip Alternate Data Streams (name:stream)
    if let Some(pos) = name.find(':') {
        name = &name[..pos];
    }

    // 4. Strip illegal Windows characters
    let mut cleaned: String = name
        .chars()
        .map(|c| match c {
            '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' => '_',
            c if (c as u32) < 32 => '_',
            c => c,
        })
        .collect();

    // 5. Strip trailing dots and spaces (Windows does not allow files ending in . or space)
    while cleaned.ends_with('.') || cleaned.ends_with(' ') {
        cleaned.pop();
    }

    // 6. Check for empty string or reserved Windows device names
    let upper = cleaned.to_ascii_uppercase();
    let is_reserved = matches!(
        upper.as_str(),
        "CON"
            | "PRN"
            | "AUX"
            | "NUL"
            | "COM1"
            | "COM2"
            | "COM3"
            | "COM4"
            | "COM5"
            | "COM6"
            | "COM7"
            | "COM8"
            | "COM9"
            | "LPT1"
            | "LPT2"
            | "LPT3"
            | "LPT4"
            | "LPT5"
            | "LPT6"
            | "LPT7"
            | "LPT8"
            | "LPT9"
    );

    if cleaned.is_empty() || cleaned == "." || cleaned == ".." || is_reserved {
        format!(
            "{}_file",
            if cleaned.is_empty() {
                "unnamed"
            } else {
                &cleaned
            }
        )
    } else {
        cleaned
    }
}

/// Resolves a sanitized file path safely inside the target directory.
pub fn safe_destination_path(dir: &Path, filename: &str) -> PathBuf {
    let safe_name = sanitize_filename(filename);
    dir.join(safe_name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normal_filename() {
        assert_eq!(sanitize_filename("report.pdf"), "report.pdf");
        assert_eq!(sanitize_filename("my photo 2026.png"), "my photo 2026.png");
    }

    #[test]
    fn test_path_traversal() {
        assert_eq!(sanitize_filename("../../etc/passwd"), "passwd");
        assert_eq!(
            sanitize_filename("..\\..\\Windows\\System32\\cmd.exe"),
            "cmd.exe"
        );
    }

    #[test]
    fn test_trailing_dots_and_spaces() {
        assert_eq!(sanitize_filename("test.txt..."), "test.txt");
        assert_eq!(sanitize_filename("test.txt   "), "test.txt");
        assert_eq!(sanitize_filename("test.txt. . "), "test.txt");
    }

    #[test]
    fn test_alternate_data_stream() {
        assert_eq!(sanitize_filename("file.txt:hidden"), "file.txt");
    }

    #[test]
    fn test_reserved_windows_names() {
        assert_eq!(sanitize_filename("CON"), "CON_file");
        assert_eq!(sanitize_filename("nul"), "nul_file");
        assert_eq!(sanitize_filename("COM1"), "COM1_file");
    }

    #[test]
    fn test_illegal_characters() {
        assert_eq!(
            sanitize_filename("hello<?>*|:file.txt"),
            "hello_______file.txt"
        );
    }

    #[test]
    fn test_empty_and_dots() {
        assert_eq!(sanitize_filename(""), "unnamed_file");
        assert_eq!(sanitize_filename("..."), "unnamed_file");
    }
}
