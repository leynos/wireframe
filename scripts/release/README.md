# Release scripts

These modules record a release candidate, observe crates.io, and check what a
release published. `workspace_candidate`, `release_workspace_cli`, and the
shell scripts are Wireframe's own. The rest are imported from
[`leynos/netsuke`](https://github.com/leynos/netsuke) at commit
`56d1e0a224cec931eed9b55948b5f112113aba69`, and share its licence.

| Module                                                   | Role                                                        |
| -------------------------------------------------------- | ----------------------------------------------------------- |
| `crates_io_registry_http`, `crates_io_transport_http`    | Send allowlisted requests and bound the retries             |
| `crates_io_transport_cli`                                | Record a registry probe or a downloaded archive             |
| `release_registry_response`, `release_crates_io_adapter` | Translate registry responses into typed observations        |
| `release_reconcile_crates`, `verify_crates_io_bytes`     | Decide what those observations permit                       |
| `release_digest`, `release_digest_files`                 | Hash files and compare digests                              |
| `workspace_candidate`                                    | Record the packaged archives and bind them to a commit      |
| `release_workspace_cli`                                  | Run the recorded, surveyed, and verified steps of a release |
| `consumer-smoke.sh`, `github-release.sh`                 | Build a consumer from crates.io, and announce the release   |

Netsuke uploads a retained archive to the registry itself. Here lading
publishes, so the upload command and its metadata builder are left out, and no
module takes a registry credential. The checksum-sidecar code is left out too,
because it describes binary assets, which this library does not ship.

Run the tests with `make test-release-scripts`. The modules need Python 3.14.
