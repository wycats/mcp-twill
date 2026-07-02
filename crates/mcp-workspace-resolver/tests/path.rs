//! Path normalization and boundary check tests (RFC 0007 case rules).

use mcp_workspace_resolver::{normalize_file_uri, normalize_path, path_has_prefix, paths_equal};

fn has_prefix(candidate: &str, root: &str) -> bool {
    path_has_prefix(&normalize_path(candidate), &normalize_path(root))
}

#[test]
fn posix_paths_are_case_sensitive() {
    assert!(has_prefix("/repo/src/main.rs", "/repo"));
    assert!(!has_prefix("/Repo/src/main.rs", "/repo"));
    assert!(!has_prefix("/repo/src/main.rs", "/Repo"));
}

#[test]
fn drive_letter_paths_are_case_insensitive() {
    assert!(has_prefix("C:/Repo/src/main.rs", "c:/repo"));
    assert!(has_prefix("c:/repo/src", "C:/Repo"));
    assert!(!has_prefix("D:/repo/src", "C:/repo"));
}

#[test]
fn backslash_separators_normalize() {
    assert!(has_prefix(r"C:\Repo\src\main.rs", "c:/repo"));
}

#[test]
fn traversal_escaping_the_root_is_rejected() {
    assert!(!has_prefix("/repo/../etc/passwd", "/repo"));
    assert!(!has_prefix("/repo/src/../../etc", "/repo"));
    assert!(has_prefix("/repo/src/../docs", "/repo"));
}

#[test]
fn root_is_inside_itself() {
    assert!(has_prefix("/repo", "/repo"));
    assert!(has_prefix("/repo/", "/repo"));
}

#[test]
fn drive_and_driveless_paths_never_contain_each_other() {
    assert!(!has_prefix("C:/repo/src", "/repo"));
    assert!(!has_prefix("/repo/src", "C:/repo"));
}

#[test]
fn relative_candidate_is_not_inside_absolute_root() {
    assert!(!has_prefix("repo/src", "/repo"));
}

#[test]
fn file_uri_normalization_strips_scheme() {
    let uri = normalize_file_uri("file:///workspace/project").expect("file uri");
    let path = normalize_path("/workspace/project");
    assert!(paths_equal(&uri, &path));
}

#[test]
fn file_uri_with_drive_letter_normalizes() {
    let uri = normalize_file_uri("file:///C:/Repo").expect("file uri");
    let path = normalize_path("c:/repo");
    assert!(paths_equal(&uri, &path));
}

#[test]
fn plain_path_is_accepted_by_normalize_file_uri() {
    let plain = normalize_file_uri("/workspace/project").expect("plain path");
    assert!(paths_equal(&plain, &normalize_path("/workspace/project")));
}

#[test]
fn windows_drive_path_is_not_mistaken_for_a_scheme() {
    let drive = normalize_file_uri("C:/repo").expect("drive path");
    assert_eq!(drive.drive(), Some('C'));
}

#[test]
fn non_file_scheme_is_rejected() {
    let err = normalize_file_uri("https://example.com/repo").expect_err("rejected");
    assert_eq!(err.scheme, "https");
    let err = normalize_file_uri("vscode-remote://wsl/repo").expect_err("rejected");
    assert_eq!(err.scheme, "vscode-remote");
}

#[test]
fn dot_segments_resolve_lexically() {
    let a = normalize_path("/workspace/./project/../project/src");
    let b = normalize_path("/workspace/project/src");
    assert!(paths_equal(&a, &b));
}
