mod constants;
mod formatter;
mod function_collector;
#[cfg(test)]
mod function_length_fixture;
mod function_length_plan;

#[cfg(test)]
pub(super) use function_length_fixture::FunctionLengthFixture;
pub(super) use function_length_plan::FunctionLengthPlan;
