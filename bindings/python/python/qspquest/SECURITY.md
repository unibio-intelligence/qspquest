# Security policy

Report suspected QSPQuest vulnerabilities privately to
[helpdesk@unibiointelligence.com](mailto:helpdesk@unibiointelligence.com).
Include the affected package or component, exact release version or source
revision, potential impact and minimal reproduction steps using synthetic inputs.
Keep vulnerability details out of public issues while maintainers assess the
report and coordinate a fix or mitigation.

Remove credentials, signing keys, confidential study inputs and personal data
from reports and logs. If sensitive evidence is necessary, request a secure
transfer method before sending it. A patch is welcome but is not required.
Use the ordinary issue tracker for nonsensitive bugs.

Maintainers prioritize the current official release. Identify older affected
versions when known so their exposure and remediation can be assessed. A fix
is available to users only after a corrected release is published; a source
change or successful test alone does not establish that a deployed service or
installed package has been updated.

Maintainers review security-sensitive changes and release inputs. A temporary
dependency or license exception must identify an owner, the exact finding and
component/version, its source or artifact digest, rationale, compensating
controls, dated review evidence and an expiry. Exceptions cannot waive secret
redaction, required artifact verification or publication review.

Follow `docs/release-verification.md` in the matching complete QSPQuest source
release to check an artifact's digest, expected build identity and signed SBOM
before installation.
