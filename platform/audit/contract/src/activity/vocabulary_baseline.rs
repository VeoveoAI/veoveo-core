//! Frozen vocabulary formats captured from the declarations before derive migration.
mod baseline {
    #[derive(
        Debug,
        Clone,
        Copy,
        PartialEq,
        Eq,
        serde::Serialize,
        serde::Deserialize,
        schemars::JsonSchema,
    )]
    #[serde(rename_all = "snake_case")]
    pub enum AuditOutcome {
        Allowed,
        Denied,
        Succeeded,
        Failed,
    }
    #[derive(
        Debug,
        Clone,
        Copy,
        PartialEq,
        Eq,
        serde::Serialize,
        serde::Deserialize,
        schemars::JsonSchema,
    )]
    #[serde(rename_all = "snake_case")]
    pub enum AuditReason {
        Accepted,
        PolicyDenied,
        TenantMismatch,
        InsufficientClearance,
        InsufficientAccess,
        Unauthenticated,
        InvalidCredential,
        ExpiredCredential,
        Replay,
        Revoked,
        InvalidRequest,
        NotFound,
        Conflict,
        Unavailable,
        QuotaExceeded,
        Cancelled,
        TimedOut,
        UpstreamFailure,
        InternalFailure,
        UnknownServer,
        UnknownResource,
        UnknownTask,
        UnknownPrincipal,
        UnknownScope,
        UnknownDataLabel,
        UnknownTenant,
        UnknownTokenIssuer,
        MissingPrincipal,
        MissingTenant,
        MissingGroup,
        MissingRole,
        MissingScope,
        MissingDataLabel,
        MissingPrincipalAssurance,
        TokenAudienceMismatch,
        TokenNotYetValid,
    }
}
fn compare<
    A: serde::Serialize + for<'de> serde::Deserialize<'de> + PartialEq + std::fmt::Debug,
    B: serde::Serialize,
>(
    actual: A,
    before: B,
) {
    let wire = serde_json::to_value(&before).unwrap();
    assert_eq!(serde_json::to_value(&actual).unwrap(), wire);
    assert_eq!(serde_json::from_value::<A>(wire).unwrap(), actual);
    let token = before.serialize(UnitVariantProbe).unwrap();
    assert_eq!(actual.serialize(UnitVariantProbe).unwrap(), token);
    let decoded = A::deserialize(serde::de::value::EnumAccessDeserializer::new(Ordinal(
        token.1,
    )))
    .unwrap();
    assert_eq!(decoded, actual);
}
#[test]
fn frozen_activity_vocabulary_wire_and_schemas() {
    compare(
        super::AuditOutcome::Allowed,
        baseline::AuditOutcome::Allowed,
    );
    compare(super::AuditOutcome::Denied, baseline::AuditOutcome::Denied);
    compare(
        super::AuditOutcome::Succeeded,
        baseline::AuditOutcome::Succeeded,
    );
    compare(super::AuditOutcome::Failed, baseline::AuditOutcome::Failed);
    assert_eq!(
        schemars::schema_for!(super::AuditOutcome),
        schemars::schema_for!(baseline::AuditOutcome)
    );
    compare(
        super::AuditReason::Accepted,
        baseline::AuditReason::Accepted,
    );
    compare(
        super::AuditReason::PolicyDenied,
        baseline::AuditReason::PolicyDenied,
    );
    compare(
        super::AuditReason::TenantMismatch,
        baseline::AuditReason::TenantMismatch,
    );
    compare(
        super::AuditReason::InsufficientClearance,
        baseline::AuditReason::InsufficientClearance,
    );
    compare(
        super::AuditReason::InsufficientAccess,
        baseline::AuditReason::InsufficientAccess,
    );
    compare(
        super::AuditReason::Unauthenticated,
        baseline::AuditReason::Unauthenticated,
    );
    compare(
        super::AuditReason::InvalidCredential,
        baseline::AuditReason::InvalidCredential,
    );
    compare(
        super::AuditReason::ExpiredCredential,
        baseline::AuditReason::ExpiredCredential,
    );
    compare(super::AuditReason::Replay, baseline::AuditReason::Replay);
    compare(super::AuditReason::Revoked, baseline::AuditReason::Revoked);
    compare(
        super::AuditReason::InvalidRequest,
        baseline::AuditReason::InvalidRequest,
    );
    compare(
        super::AuditReason::NotFound,
        baseline::AuditReason::NotFound,
    );
    compare(
        super::AuditReason::Conflict,
        baseline::AuditReason::Conflict,
    );
    compare(
        super::AuditReason::Unavailable,
        baseline::AuditReason::Unavailable,
    );
    compare(
        super::AuditReason::QuotaExceeded,
        baseline::AuditReason::QuotaExceeded,
    );
    compare(
        super::AuditReason::Cancelled,
        baseline::AuditReason::Cancelled,
    );
    compare(
        super::AuditReason::TimedOut,
        baseline::AuditReason::TimedOut,
    );
    compare(
        super::AuditReason::UpstreamFailure,
        baseline::AuditReason::UpstreamFailure,
    );
    compare(
        super::AuditReason::InternalFailure,
        baseline::AuditReason::InternalFailure,
    );
    compare(
        super::AuditReason::UnknownServer,
        baseline::AuditReason::UnknownServer,
    );
    compare(
        super::AuditReason::UnknownResource,
        baseline::AuditReason::UnknownResource,
    );
    compare(
        super::AuditReason::UnknownTask,
        baseline::AuditReason::UnknownTask,
    );
    compare(
        super::AuditReason::UnknownPrincipal,
        baseline::AuditReason::UnknownPrincipal,
    );
    compare(
        super::AuditReason::UnknownScope,
        baseline::AuditReason::UnknownScope,
    );
    compare(
        super::AuditReason::UnknownDataLabel,
        baseline::AuditReason::UnknownDataLabel,
    );
    compare(
        super::AuditReason::UnknownTenant,
        baseline::AuditReason::UnknownTenant,
    );
    compare(
        super::AuditReason::UnknownTokenIssuer,
        baseline::AuditReason::UnknownTokenIssuer,
    );
    compare(
        super::AuditReason::MissingPrincipal,
        baseline::AuditReason::MissingPrincipal,
    );
    compare(
        super::AuditReason::MissingTenant,
        baseline::AuditReason::MissingTenant,
    );
    compare(
        super::AuditReason::MissingGroup,
        baseline::AuditReason::MissingGroup,
    );
    compare(
        super::AuditReason::MissingRole,
        baseline::AuditReason::MissingRole,
    );
    compare(
        super::AuditReason::MissingScope,
        baseline::AuditReason::MissingScope,
    );
    compare(
        super::AuditReason::MissingDataLabel,
        baseline::AuditReason::MissingDataLabel,
    );
    compare(
        super::AuditReason::MissingPrincipalAssurance,
        baseline::AuditReason::MissingPrincipalAssurance,
    );
    compare(
        super::AuditReason::TokenAudienceMismatch,
        baseline::AuditReason::TokenAudienceMismatch,
    );
    compare(
        super::AuditReason::TokenNotYetValid,
        baseline::AuditReason::TokenNotYetValid,
    );
    assert_eq!(
        schemars::schema_for!(super::AuditReason),
        schemars::schema_for!(baseline::AuditReason)
    );
}

/// Observe Serde's nonhuman unit-variant identity and ordinal without another codec.
struct UnitVariantProbe;
type ProbeError = serde::de::value::Error;
type ProbeToken = (&'static str, u32, &'static str);
impl serde::Serializer for UnitVariantProbe {
    type Ok = ProbeToken;
    type Error = ProbeError;
    type SerializeSeq = serde::ser::Impossible<ProbeToken, ProbeError>;
    type SerializeTuple = serde::ser::Impossible<ProbeToken, ProbeError>;
    type SerializeTupleStruct = serde::ser::Impossible<ProbeToken, ProbeError>;
    type SerializeTupleVariant = serde::ser::Impossible<ProbeToken, ProbeError>;
    type SerializeMap = serde::ser::Impossible<ProbeToken, ProbeError>;
    type SerializeStruct = serde::ser::Impossible<ProbeToken, ProbeError>;
    type SerializeStructVariant = serde::ser::Impossible<ProbeToken, ProbeError>;
    fn is_human_readable(&self) -> bool {
        false
    }
    fn serialize_unit_variant(
        self,
        name: &'static str,
        index: u32,
        variant: &'static str,
    ) -> Result<Self::Ok, Self::Error> {
        Ok((name, index, variant))
    }
    fn serialize_bool(self, _: bool) -> Result<Self::Ok, Self::Error> {
        Err(serde::ser::Error::custom("expected unit variant"))
    }
    fn serialize_i8(self, _: i8) -> Result<Self::Ok, Self::Error> {
        Err(serde::ser::Error::custom("expected unit variant"))
    }
    fn serialize_i16(self, _: i16) -> Result<Self::Ok, Self::Error> {
        Err(serde::ser::Error::custom("expected unit variant"))
    }
    fn serialize_i32(self, _: i32) -> Result<Self::Ok, Self::Error> {
        Err(serde::ser::Error::custom("expected unit variant"))
    }
    fn serialize_i64(self, _: i64) -> Result<Self::Ok, Self::Error> {
        Err(serde::ser::Error::custom("expected unit variant"))
    }
    fn serialize_i128(self, _: i128) -> Result<Self::Ok, Self::Error> {
        Err(serde::ser::Error::custom("expected unit variant"))
    }
    fn serialize_u8(self, _: u8) -> Result<Self::Ok, Self::Error> {
        Err(serde::ser::Error::custom("expected unit variant"))
    }
    fn serialize_u16(self, _: u16) -> Result<Self::Ok, Self::Error> {
        Err(serde::ser::Error::custom("expected unit variant"))
    }
    fn serialize_u32(self, _: u32) -> Result<Self::Ok, Self::Error> {
        Err(serde::ser::Error::custom("expected unit variant"))
    }
    fn serialize_u64(self, _: u64) -> Result<Self::Ok, Self::Error> {
        Err(serde::ser::Error::custom("expected unit variant"))
    }
    fn serialize_u128(self, _: u128) -> Result<Self::Ok, Self::Error> {
        Err(serde::ser::Error::custom("expected unit variant"))
    }
    fn serialize_f32(self, _: f32) -> Result<Self::Ok, Self::Error> {
        Err(serde::ser::Error::custom("expected unit variant"))
    }
    fn serialize_f64(self, _: f64) -> Result<Self::Ok, Self::Error> {
        Err(serde::ser::Error::custom("expected unit variant"))
    }
    fn serialize_char(self, _: char) -> Result<Self::Ok, Self::Error> {
        Err(serde::ser::Error::custom("expected unit variant"))
    }
    fn serialize_str(self, _: &str) -> Result<Self::Ok, Self::Error> {
        Err(serde::ser::Error::custom("expected unit variant"))
    }
    fn serialize_bytes(self, _: &[u8]) -> Result<Self::Ok, Self::Error> {
        Err(serde::ser::Error::custom("expected unit variant"))
    }
    fn serialize_none(self) -> Result<Self::Ok, Self::Error> {
        Err(serde::ser::Error::custom("expected unit variant"))
    }
    fn serialize_unit(self) -> Result<Self::Ok, Self::Error> {
        Err(serde::ser::Error::custom("expected unit variant"))
    }
    fn serialize_unit_struct(self, _: &'static str) -> Result<Self::Ok, Self::Error> {
        Err(serde::ser::Error::custom("expected unit variant"))
    }
    fn serialize_seq(self, _: Option<usize>) -> Result<Self::SerializeSeq, Self::Error> {
        Err(serde::ser::Error::custom("expected unit variant"))
    }
    fn serialize_tuple(self, _: usize) -> Result<Self::SerializeTuple, Self::Error> {
        Err(serde::ser::Error::custom("expected unit variant"))
    }
    fn serialize_tuple_struct(
        self,
        _: &'static str,
        _: usize,
    ) -> Result<Self::SerializeTupleStruct, Self::Error> {
        Err(serde::ser::Error::custom("expected unit variant"))
    }
    fn serialize_tuple_variant(
        self,
        _: &'static str,
        _: u32,
        _: &'static str,
        _: usize,
    ) -> Result<Self::SerializeTupleVariant, Self::Error> {
        Err(serde::ser::Error::custom("expected unit variant"))
    }
    fn serialize_map(self, _: Option<usize>) -> Result<Self::SerializeMap, Self::Error> {
        Err(serde::ser::Error::custom("expected unit variant"))
    }
    fn serialize_struct(
        self,
        _: &'static str,
        _: usize,
    ) -> Result<Self::SerializeStruct, Self::Error> {
        Err(serde::ser::Error::custom("expected unit variant"))
    }
    fn serialize_struct_variant(
        self,
        _: &'static str,
        _: u32,
        _: &'static str,
        _: usize,
    ) -> Result<Self::SerializeStructVariant, Self::Error> {
        Err(serde::ser::Error::custom("expected unit variant"))
    }
    fn serialize_some<T: serde::Serialize + ?Sized>(self, _: &T) -> Result<Self::Ok, Self::Error> {
        Err(serde::ser::Error::custom("expected unit variant"))
    }
    fn serialize_newtype_struct<T: serde::Serialize + ?Sized>(
        self,
        _: &'static str,
        _: &T,
    ) -> Result<Self::Ok, Self::Error> {
        Err(serde::ser::Error::custom("expected unit variant"))
    }
    fn serialize_newtype_variant<T: serde::Serialize + ?Sized>(
        self,
        _: &'static str,
        _: u32,
        _: &'static str,
        _: &T,
    ) -> Result<Self::Ok, Self::Error> {
        Err(serde::ser::Error::custom("expected unit variant"))
    }
}
struct Ordinal(u32);
impl<'de> serde::de::EnumAccess<'de> for Ordinal {
    type Error = ProbeError;
    type Variant = Self;
    fn variant_seed<V: serde::de::DeserializeSeed<'de>>(
        self,
        seed: V,
    ) -> Result<(V::Value, Self), Self::Error> {
        let value =
            seed.deserialize(serde::de::value::U32Deserializer::<ProbeError>::new(self.0))?;
        Ok((value, self))
    }
}
impl<'de> serde::de::VariantAccess<'de> for Ordinal {
    type Error = ProbeError;
    fn unit_variant(self) -> Result<(), Self::Error> {
        Ok(())
    }
    fn newtype_variant_seed<T: serde::de::DeserializeSeed<'de>>(
        self,
        _: T,
    ) -> Result<T::Value, Self::Error> {
        Err(serde::de::Error::custom("expected unit variant"))
    }
    fn tuple_variant<V: serde::de::Visitor<'de>>(
        self,
        _: usize,
        _: V,
    ) -> Result<V::Value, Self::Error> {
        Err(serde::de::Error::custom("expected unit variant"))
    }
    fn struct_variant<V: serde::de::Visitor<'de>>(
        self,
        _: &'static [&'static str],
        _: V,
    ) -> Result<V::Value, Self::Error> {
        Err(serde::de::Error::custom("expected unit variant"))
    }
}
