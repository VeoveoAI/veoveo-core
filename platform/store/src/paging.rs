//! Keyset pages: a page size the caller's contract caps, the page that a read with
//! `LIMIT size + 1` produces, and an opaque cursor bound to the scope that issued it.
use std::{fmt, marker::PhantomData, num::NonZeroU32};

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use sha2::{Digest, Sha256};
use thiserror::Error;
use veoveo_types::{CursorCodec, OpaqueCursor, Sha256Digest};

/// The hash domain of a cursor scope, followed by a NUL byte in the digest input.
const SCOPE_FORMAT: &str = "https://veoveo.ai/formats/keyset-cursor-scope";
/// Wire cursors longer than this are refused before decoding.
const MAX_CURSOR_BYTES: usize = 4096;

/// A page size from 1 to `MAX` rows. The contract that serves the listing chooses `MAX`,
/// so `PageSize<1000>` and `PageSize<100>` are different types.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PageSize<const MAX: u32>(NonZeroU32);

impl<const MAX: u32> PageSize<MAX> {
    const VALID: () = assert!(MAX >= 1, "a page holds at least one row");

    /// Admit a requested size, refusing zero and sizes above `MAX`.
    pub fn new(requested: u32) -> Result<Self, PageSizeError> {
        let () = Self::VALID;
        match NonZeroU32::new(requested) {
            Some(size) if requested <= MAX => Ok(Self(size)),
            _ => Err(PageSizeError {
                requested,
                max: MAX,
            }),
        }
    }

    /// The requested size lowered to `MAX`, for contracts that cap rather than refuse.
    pub fn capped(requested: NonZeroU32) -> Self {
        let () = Self::VALID;
        Self(requested.min(NonZeroU32::new(MAX).expect("MAX is at least one")))
    }

    /// The largest page the contract allows.
    pub fn max() -> Self {
        let () = Self::VALID;
        Self(NonZeroU32::new(MAX).expect("MAX is at least one"))
    }

    pub fn get(self) -> u32 {
        self.0.get()
    }

    /// The `LIMIT` a page read binds: one row more than the page holds, so the extra row
    /// shows that another page follows without a second query.
    pub fn fetch_limit(self) -> i64 {
        i64::from(self.0.get()) + 1
    }

    /// The page from rows read with [`Self::fetch_limit`]. When the extra row came back,
    /// the page drops it and `next` holds the position of the last row kept.
    pub fn page<T, P>(self, mut rows: Vec<T>, position: impl FnOnce(&T) -> P) -> Page<T, P> {
        let size = self.0.get() as usize;
        if rows.len() <= size {
            return Page {
                items: rows,
                next: None,
            };
        }
        rows.truncate(size);
        let next = rows.last().map(position);
        Page { items: rows, next }
    }
}

impl<const MAX: u32> fmt::Display for PageSize<MAX> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

/// A requested page size outside `1..=max`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[error("a page holds 1 to {max} rows, not {requested}")]
pub struct PageSizeError {
    pub requested: u32,
    pub max: u32,
}

/// One page of a listing and where the next page starts, if one follows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Page<T, N> {
    pub items: Vec<T>,
    pub next: Option<N>,
}

impl<T, N> Page<T, N> {
    /// Turn the next position into another form, such as a [`KeysetCursor`].
    pub fn try_map_next<M, E>(self, map: impl FnOnce(N) -> Result<M, E>) -> Result<Page<T, M>, E> {
        Ok(Page {
            items: self.items,
            next: self.next.map(map).transpose()?,
        })
    }
}

/// What a cursor is bound to: the listing that issued it, the tenant it read and the
/// filters that selected its rows. A cursor parses only under the same scope, so a
/// cursor from another tenant, listing or filter is refused instead of paging the
/// wrong rows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CursorScope(Sha256Digest);

impl CursorScope {
    /// `listing` names the read, such as `tasks.owner_page`. `filter` holds every request
    /// value besides the cursor that shapes the result, serialized as JSON; a struct or
    /// a `BTreeMap` keeps that serialization stable.
    pub fn new(
        listing: &str,
        tenant: impl fmt::Display,
        filter: &impl Serialize,
    ) -> Result<Self, CursorError> {
        let filter = serde_json::to_vec(filter).map_err(|_| CursorError::Encoding)?;
        let tenant = tenant.to_string();
        let mut digest = Sha256::new();
        digest.update(SCOPE_FORMAT.as_bytes());
        digest.update([0]);
        for part in [listing.as_bytes(), tenant.as_bytes(), filter.as_slice()] {
            digest.update((part.len() as u64).to_be_bytes());
            digest.update(part);
        }
        Ok(Self(Sha256Digest::from_bytes(digest.finalize().into())))
    }

    /// A cursor at `position` for this scope.
    pub fn cursor<P: Serialize + DeserializeOwned>(
        &self,
        position: P,
    ) -> Result<KeysetCursor<P>, CursorError> {
        OpaqueCursor::try_new(KeysetCodec::new(self.clone()), position)
    }

    /// Admit a cursor a client returned, refusing one that another scope issued.
    pub fn parse<P: Serialize + DeserializeOwned>(
        &self,
        wire: impl Into<String>,
    ) -> Result<KeysetCursor<P>, CursorError> {
        OpaqueCursor::parse(KeysetCodec::new(self.clone()), wire)
    }
}

/// An opaque keyset cursor: the last position of a page and the scope that issued it.
pub type KeysetCursor<P> = OpaqueCursor<KeysetCodec<P>>;

/// The codec of [`KeysetCursor`]: base64url without padding over a JSON object holding
/// the scope digest and the position. Decoding accepts only the canonical encoding.
pub struct KeysetCodec<P> {
    scope: CursorScope,
    position: PhantomData<fn() -> P>,
}

impl<P> KeysetCodec<P> {
    pub fn new(scope: CursorScope) -> Self {
        Self {
            scope,
            position: PhantomData,
        }
    }

    pub fn scope(&self) -> &CursorScope {
        &self.scope
    }
}

impl<P> Clone for KeysetCodec<P> {
    fn clone(&self) -> Self {
        Self::new(self.scope.clone())
    }
}

impl<P> PartialEq for KeysetCodec<P> {
    fn eq(&self, other: &Self) -> bool {
        self.scope == other.scope
    }
}

impl<P> Eq for KeysetCodec<P> {}

impl<P> fmt::Debug for KeysetCodec<P> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("KeysetCodec")
            .field("scope", &self.scope)
            .finish()
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct EncodedCursor<'a, P> {
    scope: &'a Sha256Digest,
    position: &'a P,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct DecodedCursor<P> {
    scope: Sha256Digest,
    position: P,
}

impl<P: Serialize + DeserializeOwned> CursorCodec for KeysetCodec<P> {
    type Position = P;
    type Error = CursorError;

    fn check(&self, _position: &P) -> Result<(), CursorError> {
        Ok(())
    }

    fn encode(&self, position: &P) -> Result<String, CursorError> {
        let document = EncodedCursor {
            scope: &self.scope.0,
            position,
        };
        let json = serde_json::to_vec(&document).map_err(|_| CursorError::Encoding)?;
        let wire = URL_SAFE_NO_PAD.encode(json);
        if wire.len() > MAX_CURSOR_BYTES {
            return Err(CursorError::Encoding);
        }
        Ok(wire)
    }

    fn decode(&self, wire: &str) -> Result<P, CursorError> {
        if wire.len() > MAX_CURSOR_BYTES {
            return Err(CursorError::Malformed);
        }
        let json = URL_SAFE_NO_PAD
            .decode(wire)
            .map_err(|_| CursorError::Malformed)?;
        let decoded: DecodedCursor<P> =
            serde_json::from_slice(&json).map_err(|_| CursorError::Malformed)?;
        if decoded.scope != self.scope.0 {
            return Err(CursorError::Scope);
        }
        if self.encode(&decoded.position)? != wire {
            return Err(CursorError::Malformed);
        }
        Ok(decoded.position)
    }
}

/// A cursor that could not be issued or admitted. Messages never repeat the cursor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum CursorError {
    #[error("the cursor is not one this listing issued")]
    Malformed,
    #[error("the cursor was issued for another tenant, listing or filter")]
    Scope,
    #[error("the cursor position or filter does not serialize as JSON within 4096 bytes")]
    Encoding,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    struct Position {
        created_at: i64,
        id: u32,
    }

    #[derive(Serialize)]
    struct Filter<'a> {
        status: Option<&'a str>,
    }

    fn scope(tenant: &str, status: Option<&str>) -> CursorScope {
        CursorScope::new("fixture.page", tenant, &Filter { status }).unwrap()
    }

    #[test]
    fn page_sizes_stay_within_the_contract_maximum() {
        assert_eq!(PageSize::<100>::new(100).unwrap().get(), 100);
        assert_eq!(
            PageSize::<100>::new(101).unwrap_err(),
            PageSizeError {
                requested: 101,
                max: 100
            }
        );
        assert!(PageSize::<100>::new(0).is_err());
        assert_eq!(
            PageSize::<100>::capped(NonZeroU32::new(500).unwrap()).get(),
            100
        );
        assert_eq!(PageSize::<100>::max().fetch_limit(), 101);
    }

    #[test]
    fn the_extra_row_becomes_the_next_position() {
        let size = PageSize::<10>::new(2).unwrap();
        let page = size.page(vec![1, 2, 3], |last| *last);
        assert_eq!(page.items, vec![1, 2]);
        assert_eq!(page.next, Some(2));
        let last = size.page(vec![1, 2], |last| *last);
        assert_eq!(last.items, vec![1, 2]);
        assert_eq!(last.next, None);
    }

    #[test]
    fn cursors_round_trip_only_in_their_own_scope() {
        let position = Position {
            created_at: 1_700_000_000,
            id: 7,
        };
        let issued = scope("acme", None).cursor(position.clone()).unwrap();
        let wire = issued.as_str().to_owned();
        let parsed = scope("acme", None).parse::<Position>(wire.clone()).unwrap();
        assert_eq!(parsed.position(), &position);
        assert_eq!(
            scope("globex", None)
                .parse::<Position>(wire.clone())
                .unwrap_err(),
            CursorError::Scope
        );
        assert_eq!(
            scope("acme", Some("failed"))
                .parse::<Position>(wire.clone())
                .unwrap_err(),
            CursorError::Scope
        );
        let other_listing = CursorScope::new("fixture.other", "acme", &Filter { status: None });
        assert_eq!(
            other_listing.unwrap().parse::<Position>(wire).unwrap_err(),
            CursorError::Scope
        );
    }

    #[test]
    fn cursors_refuse_altered_and_oversized_wires() {
        let scope = scope("acme", None);
        let wire = scope
            .cursor(Position {
                created_at: 1,
                id: 1,
            })
            .unwrap()
            .into_wire();
        let json = URL_SAFE_NO_PAD.decode(&wire).unwrap();
        let mut document: serde_json::Value = serde_json::from_slice(&json).unwrap();
        let spaced = URL_SAFE_NO_PAD.encode(serde_json::to_vec_pretty(&document).unwrap());
        assert_eq!(
            scope.parse::<Position>(spaced).unwrap_err(),
            CursorError::Malformed
        );
        document["extra"] = serde_json::Value::Bool(true);
        let extended = URL_SAFE_NO_PAD.encode(serde_json::to_vec(&document).unwrap());
        assert_eq!(
            scope.parse::<Position>(extended).unwrap_err(),
            CursorError::Malformed
        );
        assert_eq!(
            scope.parse::<Position>("not base64!").unwrap_err(),
            CursorError::Malformed
        );
        assert_eq!(
            scope.parse::<Position>("A".repeat(5000)).unwrap_err(),
            CursorError::Malformed
        );
    }
}
