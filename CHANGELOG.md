# Changelog

## [0.11.2](https://github.com/seuros/upkg/compare/upkg-v0.11.1...upkg-v0.11.2) (2026-07-06)


### Bug Fixes

* **ci:** bump pinned rust-toolchain to 1.96.1 for rama 0.3.0-rc.1 MSRV ([b3b33dd](https://github.com/seuros/upkg/commit/b3b33dd1ea993db36a5da6b42ea89669d971deb3))

## [0.11.1](https://github.com/seuros/upkg/compare/upkg-v0.11.0...upkg-v0.11.1) (2026-07-06)


### Bug Fixes

* dedupe identical if/else-if arms in cask delete escalation ([d6b419a](https://github.com/seuros/upkg/commit/d6b419aec50989570754d1c03e784be416fb878e))

## [0.11.0](https://github.com/seuros/upkg/compare/upkg-v0.10.0...upkg-v0.11.0) (2026-06-26)


### Features

* divergent-name alias catalog ([cc2fa52](https://github.com/seuros/upkg/commit/cc2fa52f6e7b3b71b12817fa7b3316899f08fee2))

## [0.10.0](https://github.com/seuros/upkg/compare/upkg-v0.9.1...upkg-v0.10.0) (2026-06-13)


### Features

* add installer PATH setup ([ff38a0e](https://github.com/seuros/upkg/commit/ff38a0e4c88959a26d82a070436e9874994241f5))
* add upkg shaman health check ([1454bdc](https://github.com/seuros/upkg/commit/1454bdc909c8fb520f6eab596aff6a36aa659b68))
* **cask:** support pkg artifacts ([d2b6761](https://github.com/seuros/upkg/commit/d2b67610bbf0cae568254b8b4173446c07b077a5))


### Bug Fixes

* **cask:** detect dmg blobs without file extension ([f0991d9](https://github.com/seuros/upkg/commit/f0991d9189f42a22c676f6a1b65d1af40627cd11))

## [0.9.1](https://github.com/seuros/upkg/compare/upkg-v0.9.0...upkg-v0.9.1) (2026-05-28)


### Bug Fixes

* derive Debug on CommandSpec for test expect_err ([51ffb68](https://github.com/seuros/upkg/commit/51ffb6812f67f9f55786ab9bb44aa9022362175b))

## [0.9.0](https://github.com/seuros/upkg/compare/upkg-v0.8.1...upkg-v0.9.0) (2026-05-28)


### Features

* **search:** add `upkg search` command across all backends ([7a553fa](https://github.com/seuros/upkg/commit/7a553fa2295ad57a5decb97eb5216aa8e9032e72))

## [0.8.1](https://github.com/seuros/upkg/compare/upkg-v0.8.0...upkg-v0.8.1) (2026-05-27)


### Bug Fixes

* handle flat binary/app artifact shape in cask parser ([f29f004](https://github.com/seuros/upkg/commit/f29f004fcef553d3850e4f684e807c3f7364fea5))

## [0.8.0](https://github.com/seuros/upkg/compare/upkg-v0.7.2...upkg-v0.8.0) (2026-05-27)


### Features

* add CI test and manual integration workflows ([4a3a3f5](https://github.com/seuros/upkg/commit/4a3a3f54ad34b38315c79f0f4d696c8569904961))


### Bug Fixes

* pin CI toolchain to MSRV 1.95 ([686d7d4](https://github.com/seuros/upkg/commit/686d7d45476edab80018dd96404dd3a8dc3ae783))
* update all dependencies to latest compatible versions ([f15a00d](https://github.com/seuros/upkg/commit/f15a00d4f9d14eb6ba4a5517326282d2c8e11519))
* use convert for Linux imagemagick verify, dry-run --app on macOS ([42bae12](https://github.com/seuros/upkg/commit/42bae12d327301580c4eb4da1244ed0b2ad11c0b))
* use imagemagick for integration tests, add macOS --app test ([#17](https://github.com/seuros/upkg/issues/17)) ([4587962](https://github.com/seuros/upkg/commit/4587962d9abc3692c47bd1095f13644478f05e3d))

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
