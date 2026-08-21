## README diagnostic section format

Each diagnostic reference section follows this structure:

1. `<a id="help-anchor"></a>` — HTML anchor matching `DiagnosticSpec::help_anchor`
2. `### Title` — matches or closely follows `DiagnosticSpec::headline`
3. 1-3 paragraphs explaining the problem, leading with *why* not *what*
4. Before/after code block with realistic module paths (not `foo`/`bar`)
5. If auto-fixable: `` `cargo mend --fix` can rewrite these cases automatically ``

Keep section order consistent with `DiagnosticCode::ALL`.
