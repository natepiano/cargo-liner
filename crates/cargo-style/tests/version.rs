//! Check the public version command with both invocation forms.

use std::error::Error;
use std::process::Stdio;

use cargo_berth_test_support::FixtureCommand;
use cargo_berth_test_support::TestProcessGroup;

#[test]
fn version_names_the_package_and_protocol_with_both_invocation_forms() -> Result<(), Box<dyn Error>>
{
    let mut outputs = Vec::new();
    for arguments in [vec!["--version"], vec!["style", "--version"]] {
        let output = TestProcessGroup::spawn(
            FixtureCommand::new(env!("CARGO_BIN_EXE_cargo-style"))
                .args(arguments)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped()),
        )?
        .wait_with_output()?;
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let line = String::from_utf8(output.stdout)?;
        let prefix = format!("cargo-style {} protocol ", env!("CARGO_PKG_VERSION"));
        let protocol = line
            .strip_prefix(&prefix)
            .ok_or("the version line has the wrong prefix")?
            .trim_end_matches('\n')
            .parse::<u64>()?;
        assert_eq!(line, format!("{prefix}{protocol}\n"));
        outputs.push(line);
    }
    assert_eq!(outputs[0], outputs[1]);
    Ok(())
}
