/// Placeholder cost estimation until pricing tables land.
///
/// Always returns `0.0` for now. Signature matches the fields that will drive
/// real estimates (model + token counts).
pub fn estimate_cost(
    _model: Option<&str>,
    _prompt_tokens: i64,
    _completion_tokens: i64,
    _cached_tokens: i64,
) -> f64 {
    0.0
}
