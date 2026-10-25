//! The images an installation builds, each named once in its lock and pinned by digest.
use veoveo_types::{IdMetadata, IdProfile, IdProfileSpec, Sha256Digest};

use crate::ComponentLockError;

/// The longest image name.
pub const MAX_IMAGE_NAME: usize = 100;

/// The longest repository the OCI distribution reference grammar admits.
const MAX_REPOSITORY: usize = 255;

/// An image's name in the lock: lowercase words of letters and digits joined by single
/// hyphens, starting with a letter, such as `lore` or `hub-sdk-cli`.
#[veoveo_types::id(text(ImageNames))]
pub struct ImageName(String);

/// An image by its digest: `<repository>@sha256:<64 lowercase hexadecimal digits>` in the
/// OCI distribution reference grammar, with no tag, such as
/// `registry.example.com:5000/veoveo/lore@sha256:…`.
#[veoveo_types::id(text(PinnedImages))]
pub struct PinnedImage(String);

impl PinnedImage {
    /// The reference without its digest: `[domain/]path`.
    pub fn repository(&self) -> &str {
        self.parts().0
    }

    pub fn digest(&self) -> Sha256Digest {
        Sha256Digest::parse(self.parts().1).expect("an admitted image names a SHA-256 digest")
    }

    fn parts(&self) -> (&str, &str) {
        self.as_str()
            .split_once('@')
            .expect("an admitted image names its digest")
    }
}

const WORDS_PATTERN: &str = "[a-z][a-z0-9]*(-[a-z0-9]+)*";
const LABEL_PATTERN: &str = "[A-Za-z0-9](?:[A-Za-z0-9-]*[A-Za-z0-9])?";
const COMPONENT_PATTERN: &str = "[a-z0-9]+(?:(?:[._]|__|-+)[a-z0-9]+)*";

fn image_name(value: &str) -> bool {
    let mut words = value.split('-');
    let lowercase_word = |word: &str| {
        !word.is_empty()
            && word
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
    };
    value.len() <= MAX_IMAGE_NAME
        && words.next().is_some_and(|first| {
            first.starts_with(|c: char| c.is_ascii_lowercase()) && lowercase_word(first)
        })
        && words.all(lowercase_word)
}

fn pinned_image(value: &str) -> bool {
    value.split_once('@').is_some_and(|(repository, digest)| {
        is_repository(repository)
            && Sha256Digest::parse(digest)
                .is_ok_and(|digest| digest.hex().bytes().any(|b| b != b'0'))
    })
}

/// `[domain '/'] path-component ('/' path-component)*`.
fn is_repository(value: &str) -> bool {
    let components = value.split('/').collect::<Vec<_>>();
    let path = match components.as_slice() {
        [domain, path @ ..] if !path.is_empty() && is_domain(domain) => path,
        all => all,
    };
    value.len() <= MAX_REPOSITORY && path.iter().all(|component| is_path_component(component))
}

/// `domain := host [':' port]`, where `host` is dot-separated labels of letters, digits
/// and inner hyphens.
fn is_domain(value: &str) -> bool {
    let (host, port) = match value.rsplit_once(':') {
        Some((host, port)) => (host, Some(port)),
        None => (value, None),
    };
    port.is_none_or(|port| !port.is_empty() && port.bytes().all(|b| b.is_ascii_digit()))
        && host.split('.').all(|label| {
            let bytes = label.as_bytes();
            bytes.first().is_some_and(u8::is_ascii_alphanumeric)
                && bytes.last().is_some_and(u8::is_ascii_alphanumeric)
                && bytes
                    .iter()
                    .all(|b| b.is_ascii_alphanumeric() || *b == b'-')
        })
}

/// `path-component := [a-z0-9]+ (separator [a-z0-9]+)*`, `separator := [_.] | '__' | [-]+`.
fn is_path_component(value: &str) -> bool {
    let alphanumeric = |b: &u8| b.is_ascii_lowercase() || b.is_ascii_digit();
    let bytes = value.as_bytes();
    bytes.first().is_some_and(alphanumeric)
        && bytes.last().is_some_and(alphanumeric)
        && value
            .split(|c: char| c.is_ascii_lowercase() || c.is_ascii_digit())
            .all(|separator| {
                matches!(separator, "" | "." | "_" | "__") || separator.bytes().all(|b| b == b'-')
            })
}

#[doc(hidden)]
pub struct ImageNames;
impl IdProfile for ImageNames {
    type Error = ComponentLockError;
    const PROFILE: IdProfileSpec<Self::Error> = IdProfileSpec::text(|value, _| {
        image_name(value)
            .then_some(())
            .ok_or_else(|| ComponentLockError::ImageName(value.to_owned()))
    })
    .owner_schema(image_name_schema, false);
}

fn image_name_schema(_: &mut schemars::SchemaGenerator, _: IdMetadata) -> schemars::Schema {
    schemars::json_schema!({
        "type": "string",
        "pattern": format!("^{WORDS_PATTERN}$"),
        "maxLength": MAX_IMAGE_NAME,
    })
}

#[doc(hidden)]
pub struct PinnedImages;
impl IdProfile for PinnedImages {
    type Error = ComponentLockError;
    const PROFILE: IdProfileSpec<Self::Error> = IdProfileSpec::text(|value, _| {
        pinned_image(value)
            .then_some(())
            .ok_or_else(|| ComponentLockError::Image(value.to_owned()))
    })
    .owner_schema(pinned_image_schema, false);
}

fn pinned_image_schema(_: &mut schemars::SchemaGenerator, _: IdMetadata) -> schemars::Schema {
    let domain = format!("{LABEL_PATTERN}(?:\\.{LABEL_PATTERN})*(?::[0-9]+)?");
    schemars::json_schema!({
        "type": "string",
        "pattern": format!(
            "^(?:{domain}/)?{COMPONENT_PATTERN}(?:/{COMPONENT_PATTERN})*@sha256:[0-9a-f]{{64}}$"
        ),
        "not": { "pattern": "@sha256:0{64}$" },
    })
}
