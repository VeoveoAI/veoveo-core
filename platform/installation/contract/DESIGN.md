# Installation Contract

`veoveo-installation-contract` defines the documents an installation repository keeps
about the components it runs. Its one document is the component lock, which pins the
repository and Git revision of each component, so an installation starts the same
source on every host.

## Standards And Protocols

| Standard or format | Profile |
|---|---|
| JSON, RFC 8259 | The lock is one JSON object with `camelCase` members, closed at every level |
| JSON Schema 2020-12 | `schemas/component-lock.schema.json`, generated from the Rust types by Schemars 1.2 and compared with them by a test; its `$id` is the format URI |
| Veoveo format `https://veoveo.ai/formats/component-lock` | The lock's `format` member, unversioned, as the [conventions](../../../docs/CONVENTIONS.md#formats-and-schemas) require |
| RFC 3986 URIs | A repository is an `https` URL on `github.com` whose path is exactly an owner and a repository |
| Git object names | A revision is a full SHA-1 commit ID: 40 lowercase hexadecimal digits, never the all-zero ID |

## Component Lock

```json
{
  "format": "https://veoveo.ai/formats/component-lock",
  "components": [
    {
      "name": "veoveo-service-gateway",
      "repository": "https://github.com/VeoveoAI/veoveo-service-gateway",
      "revision": "0123456789abcdef0123456789abcdef01234567"
    }
  ]
}
```

`ComponentLock` admits a document with exactly the members `format` and `components`.
`components` lists at least one component, and no name appears twice. Each component has
exactly these members:

| Member | Rule |
|---|---|
| `name` | The component's repository name, `veoveo-<kind>-<name>`, at most 100 bytes. `<name>` is lowercase words of letters and digits joined by single hyphens, and its first word starts with a letter |
| `repository` | `https://github.com/<owner>/<repository>`, without a trailing slash, `.git`, query or fragment. The owner is a GitHub account name: 1 to 39 letters, digits and single inner hyphens. The repository segment equals `name` |
| `revision` | The commit to run, as `GitRevision` |

`<kind>` is one of the [repository kinds](../../../docs/CONVENTIONS.md#runtime-and-repository-names)
whose components an installation runs: `service`, `mcp`, `runtime`, `app` and
`showcase`, the `ComponentKind` vocabulary.
Libraries, templates, SDKs and qualification suites are not components, and neither
are `veoveo-core` or an organization's singleton repositories.

`ComponentName`, `ComponentRepository` and `GitRevision` admit their text once, on
construction and on decoding. `LockedComponent::new` and `ComponentLock::new` apply the
same checks as decoding. The lock keeps its components in the order the document lists
them, and `ComponentLock::component` finds one by name. The schema describes each
member's shape. Only admission checks that a repository's name equals its component's
name and that names are unique, because JSON Schema cannot compare two values.

## Crate Boundary

The installation configuration's types live in the `configuration` feature of
`veoveo-mcp-contract`, which brings the Gateway's policy, Work Context and audit
contracts. A bootstrap that reads the lock needs none of them, so the lock has a crate
of its own that depends only on `veoveo-types`, serde, schemars and thiserror.
