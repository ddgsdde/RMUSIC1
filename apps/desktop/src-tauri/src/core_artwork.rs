/// Restrict authenticated image requests to the two Core image resource families.
pub fn image_path(path: &str) -> Option<&str> {
    let path = path.trim_start_matches('/');
    let id = path.strip_prefix("api/v1/covers/").or_else(|| path.strip_prefix("api/v1/artwork/"))?;
    (id.len() == 64 && id.bytes().all(|byte| byte.is_ascii_hexdigit())).then_some(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn accepts_current_artwork_and_original_cover_resources() {
        for family in ["artwork", "covers"] {
            let path = format!("/api/v1/{family}/{}", "a".repeat(64));
            assert_eq!(image_path(&path), Some(path.trim_start_matches('/')));
        }
    }
    #[test]
    fn refuses_non_image_resources_and_path_or_query_suffixes() {
        for path in ["api/v1/auth/me".into(), "api/v1/artwork/../auth/me".into(), format!("api/v1/artwork/{}/public-fetch", "a".repeat(64)), format!("api/v1/covers/{}?ticket=secret", "a".repeat(64)), format!("api/v1/covers/{}%2fsecret", "a".repeat(64))] {
            assert_eq!(image_path(&path), None);
        }
    }
}
