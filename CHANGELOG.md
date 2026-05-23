# Changelog

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
