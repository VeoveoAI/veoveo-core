//! Veoveo JSON-RPC error codes.
//!
//! JSON-RPC 2.0 reserves `-32768` to `-32000`, and MCP assigns codes inside that range.
//! Veoveo codes come from `-31999` to `-31000`, the application range, so a new MCP code
//! cannot collide with them. Every Veoveo code is listed here and in the contract design.

/// A terminal response exceeded the serialized response budget, or could not be collected.
pub const RESPONSE_BUDGET_EXCEEDED: i64 = -31_001;

/// Every Veoveo JSON-RPC error code.
pub const ALL: &[i64] = &[RESPONSE_BUDGET_EXCEEDED];

#[cfg(test)]
mod tests {
    #[test]
    fn veoveo_codes_stay_outside_the_reserved_range() {
        for code in super::ALL {
            assert!((-31_999..=-31_000).contains(code), "{code}");
        }
    }
}
