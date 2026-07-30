//! `curl -fsSL skillrank.dev | sh` is the documented primary install path, and
//! for most people the only moment they will ever read skillrank's output.
//!
//! It used to run `skillrank setup >/dev/null 2>&1`, which threw away the one
//! line explaining that the installed Skill lets the agent consult a
//! third-party registry on its own initiative — and the one command that turns
//! that off. The Skill was enabled; nobody was told. These assertions are on
//! the shipped script text because the script itself downloads a release
//! binary, which a unit test cannot and should not do.

const INSTALL_SH: &str = include_str!("../../../install.sh");

/// Every line in the installer that runs `skillrank setup`.
fn setup_invocations() -> Vec<&'static str> {
    let lines: Vec<&str> = INSTALL_SH
        .lines()
        .map(str::trim)
        .filter(|line| !line.starts_with('#'))
        .filter(|line| line.contains("skillrank\" setup") || line.contains("skillrank setup"))
        .collect();
    assert!(
        !lines.is_empty(),
        "the installer no longer runs `skillrank setup` at all"
    );
    lines
}

#[test]
fn the_installer_does_not_discard_what_setup_says() {
    for line in setup_invocations() {
        assert!(
            !line.contains("/dev/null"),
            "the installer hides setup's output, including the trigger disclosure: {line}"
        );
    }
}

#[test]
fn the_installer_forwards_setup_output_to_the_users_terminal() {
    let forwarded = setup_invocations()
        .iter()
        .any(|line| line.contains(">&2") || line.contains("| tee"));
    assert!(
        forwarded,
        "setup's output is not routed to the stream the installer logs on:\n{:#?}",
        setup_invocations()
    );
}

/// The installer asks for the email itself. Unhiding setup's output would make
/// a second, redundant prompt visible (and blocking) on an interactive install,
/// so the empty case has to tell setup not to ask again.
#[test]
fn the_installer_does_not_leave_setup_free_to_prompt_a_second_time() {
    assert!(
        INSTALL_SH.contains("--no-email"),
        "the installer can prompt for an email twice"
    );
}
