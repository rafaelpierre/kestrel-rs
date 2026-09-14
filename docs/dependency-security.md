# Dependency security updates

Dependabot monitors the default branch's Cargo dependency graph, including
transitive dependencies recorded in `Cargo.lock`, and supported GitHub Actions
references. The intended policy opens remediation PRs for **high or critical**
Dependabot alerts when a compatible patch is available. Alerts without a patch
remain visible for maintainer review. This is advisory-based monitoring, not a
guarantee that every supply-chain vulnerability will be detected.

## Required repository settings

The YAML file alone does **not** enable alerts or severity filtering. An admin
must configure [Advanced Security](https://github.com/rafaelpierre/kestrel-rs/settings/security_analysis)
and verify these settings before treating automation as active:

1. Enable the dependency graph and **Dependabot alerts**.
2. Leave the broad **Dependabot security updates** switch **disabled**. Enabling
   it attempts fixes for all patchable alerts, bypassing the severity policy.
3. Under **Dependabot rules**, create an enabled custom rule named
   `High and critical security fix PRs`. Target severity **High OR Critical**
   and select **Open a pull request to resolve this alert**. Do not restrict
   dependency scope, package, or ecosystem; development and transitive dependencies
   need coverage too. Do not select dismissal or snoozing.
4. Check for conflicting dismissal rules or other rules that open PRs at lower
   severities. Keep unpatched alerts visible. Verify the saved rule's state,
   severity selection and action after saving.

[GitHub's custom auto-triage documentation](https://docs.github.com/en/code-security/how-tos/secure-your-supply-chain/manage-your-dependency-security/auto-triage-dependabot-alerts)
describes availability for public repositories, application to existing and future
alerts, and the interaction with the broad security-updates switch. These rules
live in GitHub settings, not `dependabot.yml`; a checkout or fork does not carry
them with it. Review the settings when transferring or recreating the repository.

## Configuration and coverage

[`.github/dependabot.yml`](../.github/dependabot.yml) configures Cargo and GitHub
Actions on the default branch. `open-pull-requests-limit: 0` disables routine
version-update PRs without disabling security PRs. The required daily schedule
is a version-update setting, not a daily security-scan guarantee. No custom
scheduled scanner, credentials, automatic merge or blanket dependency upgrade is
needed. See [configuring security updates](https://docs.github.com/en/code-security/how-tos/secure-your-supply-chain/secure-your-dependencies/configure-security-updates).

GitHub Actions alert coverage has limits: GitHub documents alerts for semantic
version references, not SHA references. Floating branches such as `@stable` are
not a promise of version-based vulnerability coverage. Actions bundled or
downloaded inside a workflow are not automatically equivalent to declared Action
dependencies. See [dependency graph ecosystem support](https://docs.github.com/en/code-security/reference/supply-chain-security/dependency-graph-supported-package-ecosystems).
Do not interpret the absence of alerts as an audit of every build tool.

## Validation and maintenance

- Confirm the saved severity rule and alerts setting in GitHub. Use an existing
  qualifying alert to verify that its linked Dependabot PR targets `main` and
  updates to a patched version. If no qualifying alert exists, record that the
  live alert-to-PR path is unverified; do not introduce a vulnerable dependency
  into this repository for a test. An isolated test repository can be used for
  a controlled demonstration.
- Check that low/moderate alerts do not trigger this rule, and that high/critical
  alerts with no compatible fix remain open. Inspect update errors and resolve
  dependency constraints manually when Dependabot cannot produce a fix.
- Existing `pull_request` CI runs formatting, Clippy, tests and benchmark
  contracts. The Rust 1.89 job checks the locked dependency graph across all
  targets/features. Dependabot PRs use read-only permissions and no Actions
  secrets; optional Honeycomb export is skipped when credentials are absent.
  See [Dependabot workflow restrictions](https://docs.github.com/en/code-security/reference/supply-chain-security/dependabot-on-actions).
- Before merging an update, require green CI, verified signatures and the
  repository's canonical ten-question evidence gate. Configure required status
  checks in branch protection if enforcement is desired; merely adding a CI
  job does not change branch protection. Do not raise the declared MSRV to
  accommodate an update without a separately scoped compatibility decision.

Related: [#171](https://github.com/rafaelpierre/kestrel-rs/issues/171).
