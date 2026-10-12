//! What a permission does to data, and what it can cost or harm.

/// Exactly one per permission.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, veoveo_types::Vocabulary)]
pub enum Access {
    Read,
    Write,
}

/// Zero or more per permission. A role grants a permission only when it allows every
/// one of its risks, so a risk a role does not name is never granted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, veoveo_types::Vocabulary)]
pub enum Risk {
    /// Reads secrets or personal data.
    Sensitive,
    Destructive,
    Billed,
    /// Acts on the real world, such as commanding a vehicle or actuator.
    Physical,
    Administer,
}
