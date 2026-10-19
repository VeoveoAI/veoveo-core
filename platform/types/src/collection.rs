//! Bounded collection construction, independent of transport and storage.
//!
//! A wire token is not an admitted query position. Only a pager parses it with
//! current listing/filter/tenant/revision context. Authorization remains the
//! caller's current query responsibility.
use crate::{
    CursorCodec, CursorError, CursorScope, KeysetCodec, OpaqueCursor, PageSize, Sha256Digest,
    StatelessCursorCodec,
};
use serde::{Deserialize, Deserializer, Serialize, Serializer, de::DeserializeOwned};
use std::{cmp::Ordering, fmt, marker::PhantomData};

mod sealed {
    pub trait Limit {}
}

/// The existing checked numeric page size is the only limit representation.
pub trait PageLimit:
    sealed::Limit
    + Copy
    + fmt::Debug
    + Serialize
    + DeserializeOwned
    + schemars::JsonSchema
    + Send
    + Sync
    + 'static
{
    const MAX: u32;
    fn new(value: u32) -> Result<Self, PagingError>;
    fn get(self) -> u32;
}
impl<const MAX: u32> sealed::Limit for PageSize<MAX> {}
impl<const MAX: u32> PageLimit for PageSize<MAX> {
    const MAX: u32 = MAX;
    fn new(value: u32) -> Result<Self, PagingError> {
        PageSize::new(value).map_err(|_| PagingError::Limit)
    }
    fn get(self) -> u32 {
        self.get()
    }
}

/// One nominal listing. The key includes its unique tie-breaker; `compare`
/// expresses the selected total order, including descending or mixed ordering.
pub trait Collection: Send + Sync + Sized + 'static {
    type Item;
    type Filters: Serialize;
    type Key: Ord + Serialize + DeserializeOwned;
    type Limit: PageLimit;
    type Policy: PagingPolicy<Self>;
    const NAME: &'static str;
    fn key(item: &Self::Item) -> Self::Key;
    fn compare(filters: &Self::Filters, left: &Self::Key, right: &Self::Key) -> Ordering {
        let _ = filters;
        left.cmp(right)
    }
}

/// Keyset consistency policy. The pager uses the native KeysetCodec encoding;
/// these policies select live or revision-bound admission, not arbitrary remote
/// continuation strategies.
pub trait PagingPolicy<C: Collection>: Send + Sync {
    fn scope(
        &self,
        tenant: impl fmt::Display,
        filters: &C::Filters,
        limit: u32,
    ) -> Result<CursorScope, CursorError>;
}

#[derive(Debug, Clone, Copy, Default)]
pub struct LiveKeyset;
impl<C: Collection> PagingPolicy<C> for LiveKeyset {
    fn scope(
        &self,
        tenant: impl fmt::Display,
        filters: &C::Filters,
        limit: u32,
    ) -> Result<CursorScope, CursorError> {
        #[derive(Serialize)]
        struct Selection<'a, F> {
            filters: &'a F,
            limit: u32,
        }
        CursorScope::new(C::NAME, tenant, &Selection { filters, limit })
    }
}

/// A changed revision causes contextual cursor parsing to fail. The owner obtains
/// this digest from the immutable revision it actually reads.
#[derive(Debug, Clone)]
pub struct RevisionBound {
    revision: Sha256Digest,
}
impl RevisionBound {
    pub fn new(revision: Sha256Digest) -> Self {
        Self { revision }
    }
}
impl<C: Collection> PagingPolicy<C> for RevisionBound {
    fn scope(
        &self,
        tenant: impl fmt::Display,
        filters: &C::Filters,
        limit: u32,
    ) -> Result<CursorScope, CursorError> {
        #[derive(Serialize)]
        struct Selection<'a, F> {
            filters: &'a F,
            limit: u32,
            revision: &'a Sha256Digest,
        }
        CursorScope::new(
            C::NAME,
            tenant,
            &Selection {
                filters,
                limit,
                revision: &self.revision,
            },
        )
    }
}

/// A family-branded bounded wire token. Deserialization checks text bounds only;
/// it cannot validate request context. Never use its position as a query key.
pub type CursorToken<C> = OpaqueCursor<TokenCodec<C>>;

pub struct TokenCodec<C: Collection>(PhantomData<fn() -> C>);
impl<C: Collection> Default for TokenCodec<C> {
    fn default() -> Self {
        Self(PhantomData)
    }
}
impl<C: Collection> Clone for TokenCodec<C> {
    fn clone(&self) -> Self {
        Self::default()
    }
}
impl<C: Collection> fmt::Debug for TokenCodec<C> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("CollectionToken")
    }
}
impl<C: Collection> PartialEq for TokenCodec<C> {
    fn eq(&self, _: &Self) -> bool {
        true
    }
}
impl<C: Collection> Eq for TokenCodec<C> {}
impl<C: Collection> CursorCodec for TokenCodec<C> {
    type Position = String;
    type Error = CursorError;
    fn check(&self, value: &String) -> Result<(), CursorError> {
        if value.is_empty() || value.len() > 4096 {
            Err(CursorError::Malformed)
        } else {
            Ok(())
        }
    }
    fn encode(&self, value: &String) -> Result<String, CursorError> {
        Ok(value.clone())
    }
    fn decode(&self, wire: &str) -> Result<String, CursorError> {
        let value = wire.to_owned();
        self.check(&value)?;
        Ok(value)
    }
}
impl<C: Collection> StatelessCursorCodec for TokenCodec<C> {}

/// The one immutable page representation. Producers obtain it from `Pager`.
///
/// ```compile_fail
/// use veoveo_types::{Collection, Page};
/// fn replace<C: Collection>(page: &mut Page<C>, items: Vec<C::Item>) { page.items = items; }
/// ```
pub struct Page<C: Collection> {
    items: Vec<C::Item>,
    limit: C::Limit,
    next: Option<CursorToken<C>>,
}
impl<C: Collection> Page<C> {
    pub fn items(&self) -> &[C::Item] {
        &self.items
    }
    pub fn limit(&self) -> C::Limit {
        self.limit
    }
    pub fn next_cursor(&self) -> Option<&CursorToken<C>> {
        self.next.as_ref()
    }
    pub fn into_items(self) -> Vec<C::Item> {
        self.items
    }
}
impl<C: Collection> Clone for Page<C>
where
    C::Item: Clone,
{
    fn clone(&self) -> Self {
        Self {
            items: self.items.clone(),
            limit: self.limit,
            next: self.next.clone(),
        }
    }
}
impl<C: Collection> fmt::Debug for Page<C>
where
    C::Item: fmt::Debug,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Page")
            .field("items", &self.items)
            .field("limit", &self.limit)
            .field("next", &self.next)
            .finish()
    }
}
impl<C: Collection> Serialize for Page<C>
where
    C::Item: Serialize,
{
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        let mut value = serializer.serialize_struct("Page", 3)?;
        value.serialize_field("items", &self.items)?;
        value.serialize_field("limit", &self.limit.get())?;
        value.serialize_field("nextCursor", &self.next)?;
        value.end()
    }
}
impl<'de, C: Collection> Deserialize<'de> for Page<C>
where
    C::Item: Deserialize<'de>,
{
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct Wire<I, N> {
            items: Vec<I>,
            limit: u32,
            next_cursor: Option<N>,
        }
        let wire = Wire::<C::Item, CursorToken<C>>::deserialize(deserializer)?;
        let limit = C::Limit::new(wire.limit).map_err(serde::de::Error::custom)?;
        if wire.items.len() > limit.get() as usize {
            return Err(serde::de::Error::custom(PagingError::Lookahead));
        }
        Ok(Self {
            items: wire.items,
            limit,
            next: wire.next_cursor,
        })
    }
}

impl<C: Collection> schemars::JsonSchema for Page<C>
where
    C::Item: schemars::JsonSchema,
{
    fn schema_name() -> std::borrow::Cow<'static, str> {
        format!(
            "{}Page",
            std::any::type_name::<C>()
                .rsplit("::")
                .next()
                .unwrap_or("Collection")
        )
        .into()
    }
    fn schema_id() -> std::borrow::Cow<'static, str> {
        format!("{}<{}>", module_path!(), std::any::type_name::<C>()).into()
    }
    fn json_schema(generator: &mut schemars::SchemaGenerator) -> schemars::Schema {
        #[derive(schemars::JsonSchema)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        #[allow(dead_code)]
        struct Shape<I> {
            items: Vec<I>,
            limit: u32,
            next_cursor: Option<String>,
        }
        let mut schema = Shape::<C::Item>::json_schema(generator);
        if let Some(properties) = schema
            .get_mut("properties")
            .and_then(serde_json::Value::as_object_mut)
        {
            properties
                .get_mut("items")
                .and_then(serde_json::Value::as_object_mut)
                .map(|items| items.insert("maxItems".into(), C::Limit::MAX.into()));
            if let Some(cursor) = properties.get_mut("nextCursor") {
                *cursor = serde_json::json!({"anyOf":[{"type":"string","minLength":1,"maxLength":4096},{"type":"null"}]});
            }
            if let Some(limit) = properties
                .get_mut("limit")
                .and_then(serde_json::Value::as_object_mut)
            {
                limit.insert("minimum".into(), 1.into());
                limit.insert("maximum".into(), C::Limit::MAX.into());
            }
        }
        schema
    }
}

#[derive(Debug, thiserror::Error)]
pub enum PagingError {
    #[error("page size is outside its collection bound")]
    Limit,
    #[error("page rows exceed the admitted lookahead bound")]
    Lookahead,
    #[error("collection keys are not strictly ordered with a unique tie-breaker")]
    Order,
    #[error("page rows do not follow the admitted cursor")]
    Position,
    #[error(transparent)]
    Cursor(#[from] CursorError),
}

/// A pager captures the effective query and admits its wire cursor once. The same
/// context provides the SQL seek position, fetch bound and issued continuation.
pub struct Pager<C: Collection> {
    filters: C::Filters,
    limit: C::Limit,
    codec: KeysetCodec<C::Key>,
    after: Option<C::Key>,
}
impl<C: Collection> Pager<C> {
    pub fn new(
        tenant: impl fmt::Display,
        filters: C::Filters,
        limit: C::Limit,
        policy: C::Policy,
        cursor: Option<&CursorToken<C>>,
    ) -> Result<Self, PagingError> {
        let scope = policy.scope(tenant, &filters, limit.get())?;
        let codec = KeysetCodec::new(scope);
        let after = cursor
            .map(|wire| {
                OpaqueCursor::parse(codec.clone(), wire.as_str())
                    .map(|cursor| cursor.into_position())
            })
            .transpose()?;
        Ok(Self {
            filters,
            limit,
            codec,
            after,
        })
    }
    pub fn filters(&self) -> &C::Filters {
        &self.filters
    }
    pub fn limit(&self) -> C::Limit {
        self.limit
    }
    pub fn after(&self) -> Option<&C::Key> {
        self.after.as_ref()
    }
    pub fn fetch_limit(&self) -> i64 {
        i64::from(self.limit.get()) + 1
    }

    /// Finish exactly the ordered `limit + 1` query. The extra row determines
    /// continuation; no caller can independently provide a continuation token.
    pub fn finish(self, mut rows: Vec<C::Item>) -> Result<Page<C>, PagingError> {
        if rows.len() > self.fetch_limit() as usize {
            return Err(PagingError::Lookahead);
        }
        let mut previous = self.after.as_ref().map(|key| (key, true));
        let keys: Vec<_> = rows.iter().map(C::key).collect();
        for key in &keys {
            if let Some((last, cursor)) = previous
                && C::compare(&self.filters, last, key) != Ordering::Less
            {
                return Err(if cursor {
                    PagingError::Position
                } else {
                    PagingError::Order
                });
            }
            previous = Some((key, false));
        }
        let next = if rows.len() > self.limit.get() as usize {
            rows.truncate(self.limit.get() as usize);
            let position = C::key(rows.last().ok_or(PagingError::Limit)?);
            let cursor = OpaqueCursor::try_new(self.codec, position)?;
            Some(CursorToken::<C>::parse(
                TokenCodec::default(),
                cursor.into_wire(),
            )?)
        } else {
            None
        };
        Ok(Page {
            items: rows,
            limit: self.limit,
            next,
        })
    }

    /// Read an already ordered immutable registry through the same admission and
    /// continuation algorithm; revision policy invalidates a changed registry.
    pub fn select(self, rows: impl IntoIterator<Item = C::Item>) -> Result<Page<C>, PagingError> {
        let size = self.fetch_limit() as usize;
        let selected = rows
            .into_iter()
            .filter(|item| {
                self.after.as_ref().is_none_or(|after| {
                    C::compare(&self.filters, after, &C::key(item)) == Ordering::Less
                })
            })
            .take(size)
            .collect();
        self.finish(selected)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Entries;
    impl Collection for Entries {
        type Item = (u32, u32);
        type Filters = bool;
        type Key = (u32, u32);
        type Limit = PageSize<2>;
        type Policy = LiveKeyset;
        const NAME: &'static str = "fixture.entries";
        fn key(item: &Self::Item) -> Self::Key {
            *item
        }
        fn compare(descending: &bool, a: &Self::Key, b: &Self::Key) -> Ordering {
            if *descending { b.cmp(a) } else { a.cmp(b) }
        }
    }
    fn pager(
        tenant: &str,
        descending: bool,
        token: Option<&CursorToken<Entries>>,
    ) -> Result<Pager<Entries>, PagingError> {
        Pager::new(
            tenant,
            descending,
            PageSize::new(2).unwrap(),
            LiveKeyset,
            token,
        )
    }
    #[test]
    fn lookahead_and_context_own_continuation() {
        let page = pager("a", false, None)
            .unwrap()
            .finish(vec![(1, 1), (1, 2), (1, 3)])
            .unwrap();
        assert_eq!(page.items(), &[(1, 1), (1, 2)]);
        let token = page.next_cursor().unwrap();
        assert!(pager("b", false, Some(token)).is_err());
        assert!(pager("a", true, Some(token)).is_err());
        let next = pager("a", false, Some(token)).unwrap();
        assert_eq!(next.after(), Some(&(1, 2)));
        let terminal = next.finish(vec![(1, 3)]).unwrap();
        assert!(terminal.next_cursor().is_none());
    }
    #[test]
    fn order_cap_and_boundary_are_constructed() {
        assert!(
            pager("a", false, None)
                .unwrap()
                .finish(vec![(1, 1), (1, 1)])
                .is_err()
        );
        assert!(
            pager("a", false, None)
                .unwrap()
                .finish(vec![(1, 2), (1, 1)])
                .is_err()
        );
        assert!(
            pager("a", false, None)
                .unwrap()
                .finish(vec![(1, 1); 4])
                .is_err()
        );
        assert!(
            pager("a", false, None)
                .unwrap()
                .finish(vec![])
                .unwrap()
                .next_cursor()
                .is_none()
        );
        assert!(
            pager("a", true, None)
                .unwrap()
                .finish(vec![(1, 2), (1, 1)])
                .unwrap()
                .next_cursor()
                .is_none()
        );
    }
    struct Models;
    impl Collection for Models {
        type Item = u32;
        type Filters = ();
        type Key = u32;
        type Limit = PageSize<2>;
        type Policy = RevisionBound;
        const NAME: &'static str = "fixture.models";
        fn key(item: &u32) -> u32 {
            *item
        }
    }
    #[test]
    fn revision_and_limit_changes_refuse_old_continuation() {
        let create = |revision, limit, token| {
            Pager::<Models>::new(
                "catalog",
                (),
                PageSize::new(limit).unwrap(),
                RevisionBound::new(Sha256Digest::from_bytes([revision; 32])),
                token,
            )
        };
        let page = create(1, 2, None).unwrap().select([1, 2, 3]).unwrap();
        assert!(create(2, 2, page.next_cursor()).is_err());
        assert!(create(1, 1, page.next_cursor()).is_err());
        assert_eq!(
            create(1, 2, page.next_cursor())
                .unwrap()
                .select([1, 2, 3])
                .unwrap()
                .items(),
            &[3]
        );
    }
    #[test]
    fn received_page_checks_bounds_without_claiming_context() {
        assert!(
            serde_json::from_value::<Page<Entries>>(
                serde_json::json!({"items":[[1,1],[1,2],[1,3]],"limit":2,"nextCursor":null})
            )
            .is_err()
        );
        let token: CursorToken<Entries> =
            serde_json::from_str("\"bounded-but-not-contextual\"").unwrap();
        assert!(pager("a", false, Some(&token)).is_err());
    }
}
