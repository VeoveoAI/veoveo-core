# Installation Contract

`veoveo-installation-contract` defines the documents an installation repository keeps
about the components it runs. Its one document is the component lock, which pins the
repository and Git revision of each component, so an installation starts the same
source on every host, and names each image the installation builds once, by digest, so
every file that uses an image takes it from one place.

## Standards And Protocols

| Standard or format | Profile |
|---|---|
| JSON, RFC 8259 | The lock is one JSON object with `camelCase` members, closed at every level |
| JSON Schema 2020-12 | `schemas/component-lock.schema.json`, generated from the Rust types by Schemars 1.2 and compared with them by a test; its `$id` is the format URI |
| Veoveo format `https://veoveo.ai/formats/component-lock` | The lock's `format` member, unversioned, as the [conventions](../../../docs/CONVENTIONS.md#formats-and-schemas) require |
| RFC 3986 URIs | A repository is an `https` URL on `github.com` whose path is exactly an owner and a repository |
| Git object names | A revision is a full SHA-1 commit ID: 40 lowercase hexadecimal digits, never the all-zero ID |
| OCI distribution references | An image is `<repository>@sha256:<digest>`: a repository in the reference grammar's `[domain/]path` form, without a tag, and a SHA-256 digest of 64 lowercase hexadecimal digits, never all zeros |

## Component Lock

```json
{
  "format": "https://veoveo.ai/formats/component-lock",
  "components": [
    {
      "name": "veoveo-hub",
      "repository": "https://github.com/VeoveoAI/veoveo-hub",
      "revision": "0123456789abcdef0123456789abcdef01234567"
    }
  ],
  "images": {
    "lore": "registry.example.com:5000/veoveo/lore@sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
  }
}
```

`ComponentLock` admits a document with the members `format` and `components` and an
optional `images`, and no other. `components` lists at least one component, and no name
appears twice. Each component has exactly these members:

| Member | Rule |
|---|---|
| `name` | The component's repository name, `veoveo-<name>`, at most 100 bytes. `<name>` is lowercase words of letters and digits joined by single hyphens, and its first word starts with a letter |
| `repository` | `https://github.com/<owner>/<repository>`, without a trailing slash, `.git`, query or fragment. The owner is a GitHub account name: 1 to 39 letters, digits and single inner hyphens. The repository segment equals `name` |
| `revision` | The commit to run, as `GitRevision` |

`images` maps each image's name to its pinned reference. A name is lowercase words of
letters and digits joined by single hyphens, starting with a letter, at most 100 bytes,
such as `lore` or `hub-sdk-cli`, and the document names each once. A reference is the
image by digest, as the OCI distribution reference grammar writes it, with no tag, so
the digest alone selects the image: `PinnedImage::repository` is the reference without
its digest and `PinnedImage::digest` the digest. A lock without `images` names no image,
so a lock written before the member existed still decodes, and a lock that names none
omits it. Upgrading an image changes its one reference here.

Repository names are lexical identifiers; they do not classify runtime roles.
The installation selects which components it runs through its native runtime
configuration. A repository name alone grants no runtime authority.

`ComponentName`, `ComponentRepository`, `GitRevision`, `ImageName` and `PinnedImage`
admit their text once, on construction and on decoding. `LockedComponent::new` and
`ComponentLock::new` apply the same checks as decoding. The lock keeps its components
in the order the document lists them, and `ComponentLock::component` finds one by name;
it keeps its images ordered by name, and `ComponentLock::image` finds one. The schema
describes each member's shape. Only admission checks that a repository's name equals
its component's name and that names are unique, components' and images' alike, because
JSON Schema cannot compare two values.

## Crate Boundary

The installation configuration's types belong to the Hub, which brings its policy,
Work Context and audit contracts. A bootstrap that reads the lock needs none of them,
so the lock has a crate of its own that depends only on `veoveo-types`, serde, schemars
and thiserror.
