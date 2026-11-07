//! Parsing and canonicalising upstream repository references.

use std::path::PathBuf;

use url::Url;

use crate::error::{Error, Result};

/// A normalized git source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Repo {
    original: String,
    url: String,
    method: String,
    host: String,
    port: Option<u16>,
    transport_user: String,
    namespace: Vec<String>,
    repo: String,
}

impl Repo {
    /// Parse a GitHub shorthand, URL, scp-like SSH address, or `file://` URL.
    pub fn parse(input: &str) -> Result<Self> {
        let input = input.trim();
        if input.is_empty() {
            return Err(Error::Invalid("empty repository".to_string()));
        }
        if input.starts_with('-') {
            return Err(Error::Invalid(format!(
                "repository must not start with '-': {input}"
            )));
        }

        if is_shorthand(input) {
            let mut parts = input.split('/');
            let owner = parts.next().unwrap_or_default().to_string();
            let repo = parts.next().unwrap_or_default().to_string();
            if owner.is_empty() || repo.is_empty() || parts.next().is_some() {
                return Err(Error::Invalid(format!(
                    "GitHub shorthand must be owner/repo: {input}"
                )));
            }
            return Self::from_parts(
                input,
                format!("https://github.com/{owner}/{repo}.git"),
                "git+https",
                "github.com".to_string(),
                None,
                "anonymous".to_string(),
                vec![owner],
                repo,
            );
        }

        if let Some((user, host, path)) = parse_scp_like(input) {
            let (namespace, repo) = split_path(&path)?;
            return Self::from_parts(
                input,
                format!("ssh://{user}@{host}/{path}"),
                "git+ssh",
                host,
                None,
                user,
                namespace,
                repo,
            );
        }

        let raw_url = if let Some(rest) = input.strip_prefix("git+") {
            rest.to_string()
        } else {
            input.to_string()
        };

        let url = Url::parse(&raw_url)
            .map_err(|e| Error::Invalid(format!("invalid repository URL {input}: {e}")))?;

        let scheme = url.scheme();
        let (method, transport_user) = match scheme {
            "https" | "http" | "git" => (format!("git+{scheme}"), "anonymous".to_string()),
            "ssh" => (
                "git+ssh".to_string(),
                if url.username().is_empty() {
                    "default".to_string()
                } else {
                    url.username().to_string()
                },
            ),
            "file" => ("git+file".to_string(), "anonymous".to_string()),
            other => {
                return Err(Error::Invalid(format!(
                    "unsupported repository scheme '{other}' in {input}"
                )));
            }
        };

        if scheme == "file" {
            let path = url
                .to_file_path()
                .map_err(|_| Error::Invalid(format!("invalid file URL: {input}")))?;
            let namespace = path
                .parent()
                .map(|p| {
                    p.components()
                        .filter_map(|c| match c {
                            std::path::Component::Normal(s) => {
                                Some(s.to_string_lossy().into_owned())
                            }
                            _ => None,
                        })
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            let repo = path
                .file_name()
                .map(|s| s.to_string_lossy().into_owned())
                .ok_or_else(|| Error::Invalid(format!("file URL has no repo name: {input}")))?;
            return Self::from_parts(
                input,
                url.to_string(),
                &method,
                "localhost".to_string(),
                None,
                transport_user,
                namespace,
                strip_git(&repo).to_string(),
            );
        }

        let host = url
            .host_str()
            .ok_or_else(|| Error::Invalid(format!("repository URL has no host: {input}")))?
            .to_ascii_lowercase();
        let port = url.port();
        let (namespace, repo) = split_path(url.path())?;
        let mut canonical = url.clone();
        canonical.set_fragment(None);
        canonical.set_query(None);

        Self::from_parts(
            input,
            canonical.to_string(),
            &method,
            host,
            port,
            transport_user,
            namespace,
            repo,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn from_parts(
        original: &str,
        url: String,
        method: &str,
        host: String,
        port: Option<u16>,
        transport_user: String,
        namespace: Vec<String>,
        repo: String,
    ) -> Result<Self> {
        let repo = strip_git(&repo).to_string();
        if repo.is_empty() {
            return Err(Error::Invalid(format!(
                "repository has no name: {original}"
            )));
        }
        Ok(Self {
            original: original.to_string(),
            url,
            method: method.to_string(),
            host,
            port,
            transport_user,
            namespace,
            repo,
        })
    }

    /// The URL handed to `git fetch`.
    pub fn url(&self) -> &str {
        &self.url
    }

    /// The user-facing source string.
    pub fn original(&self) -> &str {
        &self.original
    }

    /// The final path component, without `.git`.
    pub fn repo_name(&self) -> &str {
        &self.repo
    }

    /// Path components below the host, excluding the repo name.
    pub fn namespace(&self) -> &[String] {
        &self.namespace
    }

    /// The repository's directory inside the origin cache.
    ///
    /// `<method>/<host>%3A<port>/<transport-user>/<namespace…>/<repo>.git`
    pub fn relative_dir(&self) -> PathBuf {
        let host = match self.port {
            Some(port) if !is_default_port(&self.method, port) => {
                format!("{}%3A{}", encode(&self.host), port)
            }
            _ => encode(&self.host),
        };

        let mut path = PathBuf::from(&self.method)
            .join(host)
            .join(encode(&self.transport_user));
        if self.namespace.is_empty() {
            path.push("_");
        } else {
            for part in &self.namespace {
                path.push(encode(part));
            }
        }
        path.push(format!("{}.git", encode(&self.repo)));
        path
    }

    /// The full origin path below a cache root.
    pub fn origin_path(&self, cache_root: &std::path::Path) -> PathBuf {
        cache_root.join(self.relative_dir())
    }
}

impl std::fmt::Display for Repo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.original)
    }
}

fn is_shorthand(input: &str) -> bool {
    !input.contains("://")
        && !input.starts_with('.')
        && !input.starts_with('/')
        && !input.contains('@')
        && !input.contains(':')
        && input.matches('/').count() == 1
        && !input.starts_with('-')
}

fn parse_scp_like(input: &str) -> Option<(String, String, String)> {
    let (left, path) = input.split_once(':')?;
    if left.contains('/') || path.is_empty() || path.starts_with('/') {
        return None;
    }
    let (user, host) = left.rsplit_once('@')?;
    if user.is_empty() || host.is_empty() {
        return None;
    }
    Some((
        user.to_string(),
        host.to_ascii_lowercase(),
        path.to_string(),
    ))
}

fn split_path(path: &str) -> Result<(Vec<String>, String)> {
    let parts: Vec<String> = path
        .split('/')
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .collect();
    let Some((repo, namespace)) = parts.split_last() else {
        return Err(Error::Invalid(format!(
            "repository path has no name: {path}"
        )));
    };
    Ok((namespace.to_vec(), strip_git(repo).to_string()))
}

fn strip_git(name: &str) -> &str {
    name.strip_suffix(".git").unwrap_or(name)
}

fn is_default_port(method: &str, port: u16) -> bool {
    matches!(
        (method, port),
        ("git+https", 443) | ("git+http", 80) | ("git+ssh", 22) | ("git+git", 9418)
    )
}

/// Percent-encode every byte outside `[A-Za-z0-9._-]`, reversibly.
pub fn encode(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-') {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

/// Normalize a skill path inside a repository.
pub fn normalize_skill_path(input: &str) -> Result<String> {
    let input = input.trim();
    if input.is_empty() || input == "." {
        return Ok(".".to_string());
    }
    if input.starts_with('/') || input.starts_with('-') {
        return Err(Error::Invalid(format!("invalid skill path: {input}")));
    }
    let mut parts = Vec::new();
    for part in input.split('/') {
        if part.is_empty() || part == "." {
            continue;
        }
        if part == ".." {
            return Err(Error::Invalid(format!(
                "skill path must not contain '..': {input}"
            )));
        }
        parts.push(part);
    }
    if parts.is_empty() {
        Ok(".".to_string())
    } else {
        Ok(parts.join("/"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn relative(input: &str) -> String {
        Repo::parse(input)
            .expect("repo")
            .relative_dir()
            .to_string_lossy()
            .into_owned()
    }

    #[test]
    fn github_shorthand_names_anonymously() {
        assert_eq!(
            relative("sshine/skills"),
            "git+https/github.com/anonymous/sshine/skills.git"
        );
    }

    #[test]
    fn scp_like_ssh_records_the_transport_user() {
        assert_eq!(
            relative("git@github.com:sshine/skills.git"),
            "git+ssh/github.com/git/sshine/skills.git"
        );
    }

    #[test]
    fn https_url_is_normalized() {
        assert_eq!(
            relative("https://git.shine.town/infra/helm-vendor.git"),
            "git+https/git.shine.town/anonymous/infra/helm-vendor.git"
        );
    }

    #[test]
    fn non_default_ssh_port_is_encoded() {
        assert_eq!(
            relative("ssh://git@git.example.com:2222/group/sub/repo.git"),
            "git+ssh/git.example.com%3A2222/git/group/sub/repo.git"
        );
    }

    #[test]
    fn file_url_uses_localhost() {
        assert_eq!(
            relative("file:///home/sshine/repos/foo"),
            "git+file/localhost/anonymous/home/sshine/repos/foo.git"
        );
    }

    #[test]
    fn paths_are_reversible() {
        assert_eq!(encode("a/b"), "a%2Fb");
        assert_eq!(encode("a%2Fb"), "a%252Fb");
    }

    #[test]
    fn skill_paths_are_normalized() {
        assert_eq!(normalize_skill_path("a//b/").expect("path"), "a/b");
        assert_eq!(normalize_skill_path(".").expect("path"), ".");
        assert!(normalize_skill_path("../x").is_err());
        assert!(normalize_skill_path("/x").is_err());
    }
}
