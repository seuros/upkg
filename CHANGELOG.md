# Changelog

## [0.7.2](https://github.com/seuros/upkg/compare/upkg-v0.7.1...upkg-v0.7.2) (2026-05-24)


### Bug Fixes

* add cfg gates to silence dead_code warnings on Linux ([21dd6ca](https://github.com/seuros/upkg/commit/21dd6cab03d3deae6a6bda8cb66e1be457a1c010))

## [0.7.1](https://github.com/seuros/upkg/compare/upkg-v0.7.0...upkg-v0.7.1) (2026-05-24)


### Bug Fixes

* add missing list_spec for FreeBSD backend ([dfd27cc](https://github.com/seuros/upkg/commit/dfd27ccaa055f45aab288bd2e5e9a64d738c2eb8))

## [0.7.0](https://github.com/seuros/upkg/compare/upkg-v0.6.0...upkg-v0.7.0) (2026-05-24)


### Features

* add list command support for Windows backend ([10b7547](https://github.com/seuros/upkg/commit/10b75472ee801db3799138ce0359358141d15bc4))

## [0.6.0](https://github.com/seuros/upkg/compare/upkg-v0.5.4...upkg-v0.6.0) (2026-05-24)


### Features

* add list command support for Linux and Ravenports backends ([d3b2d0f](https://github.com/seuros/upkg/commit/d3b2d0f0a1b44a59dfe959dbbc5136b9e6165965))
* add list command support for Linux and Ravenports backends ([63e1b32](https://github.com/seuros/upkg/commit/63e1b32c95135c019879f97e1cde2bad220a0dfd))


### Bug Fixes

* harden install flow with retry, locking, privilege escalation, and Mach-O patching ([fb0660e](https://github.com/seuros/upkg/commit/fb0660e2f3591d9b6efad2d1f24f32e45039c1b7))

## [0.5.4](https://github.com/seuros/upkg/compare/upkg-v0.5.3...upkg-v0.5.4) (2026-05-24)


### Bug Fixes

* patch @@HOMEBREW_PREFIX@@ placeholders in Mach-O binaries without install_name_tool ([912488b](https://github.com/seuros/upkg/commit/912488b06adce6a86db1ae02b0c8280ead597fb9))
* resolve relative symlink targets before recursive linking ([0a5baa6](https://github.com/seuros/upkg/commit/0a5baa61acc8b3adf302a92b7f0220d1a93f9c3b))

## [0.5.3](https://github.com/seuros/upkg/compare/upkg-v0.5.2...upkg-v0.5.3) (2026-05-24)


### Bug Fixes

* fall back to closest newer bottle when no same-or-older bottle exists ([cdf3926](https://github.com/seuros/upkg/commit/cdf392637648b241d70c61353463352178d5aa7d))

## [0.5.2](https://github.com/seuros/upkg/compare/upkg-v0.5.1...upkg-v0.5.2) (2026-05-23)


### Bug Fixes

* use max_attempts field instead of builder method call ([8230ca5](https://github.com/seuros/upkg/commit/8230ca59beebe6e9e24ce08f5d39c475825b9203))

## [0.5.1](https://github.com/seuros/upkg/compare/upkg-v0.5.0...upkg-v0.5.1) (2026-05-23)


### Bug Fixes

* add cfg gates to Ravenports backend to eliminate dead_code warnings ([9fda467](https://github.com/seuros/upkg/commit/9fda46719140e3389b310b071a5dab9c2406249d))
* retry API formula fetches with exponential backoff and jitter ([d59c69f](https://github.com/seuros/upkg/commit/d59c69ff4694465f2d2818c4b834025ac758ac2e))

## [0.5.0](https://github.com/seuros/upkg/compare/upkg-v0.4.0...upkg-v0.5.0) (2026-05-23)


### Features

* add Ravenports and DragonFlyBSD backend support ([ea422c7](https://github.com/seuros/upkg/commit/ea422c744dd9970c4e7ac4490172c83b177eaa6c))
* auto-detect app casks ([b222332](https://github.com/seuros/upkg/commit/b222332c8a860d104115c0a7cf5d62b732e29dc7))
* **list:** add installed package state db ([ce22e45](https://github.com/seuros/upkg/commit/ce22e4552462bb09644d53771a48d218ece8fc80))


### Bug Fixes

* **linker:** replace same-formula links on upgrade ([9584529](https://github.com/seuros/upkg/commit/958452938618704c5917a30a46bbad849dab5309))

## [0.4.0](https://github.com/seuros/upkg/compare/upkg-v0.3.0...upkg-v0.4.0) (2026-05-02)


### Features

* add self-upgrade command ([018aac8](https://github.com/seuros/upkg/commit/018aac8d50932d66e4c1b7193926f4d450355c16))

## [0.3.0](https://github.com/seuros/upkg/compare/upkg-v0.2.1...upkg-v0.3.0) (2026-05-02)


### Features

* add cask app installs ([1ffc2f3](https://github.com/seuros/upkg/commit/1ffc2f3b3ba7016799c7f2cc572384bc3dea33a1))
* add version flag ([a2ea5ec](https://github.com/seuros/upkg/commit/a2ea5ecc46b851884d7eb3f600e91293ddcbee6a))

## [0.2.1](https://github.com/seuros/upkg/compare/upkg-v0.2.0...upkg-v0.2.1) (2026-05-02)


### Bug Fixes

* statically link xz support ([359f3f5](https://github.com/seuros/upkg/commit/359f3f521424050231668dca9fd9f011149dc3d4))

## [0.2.0](https://github.com/seuros/upkg/compare/upkg-v0.1.0...upkg-v0.2.0) (2026-05-02)


### Features

* add Linux ARM64 release artifact ([daf709a](https://github.com/seuros/upkg/commit/daf709a0761356c4d2eb96fdd56a762b5997eb64))
