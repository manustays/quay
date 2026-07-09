# [0.16.0](https://github.com/manustays/quay/compare/v0.15.0...v0.16.0) (2026-07-09)


### Bug Fixes

* guard popover re-pin against monitor-less window to stop positioner panic ([34479c1](https://github.com/manustays/quay/commit/34479c1b0bcbea8bc1b4d1819410237796dd0210))


### Features

* **radar:** agent kill/ignore/reveal ([7fcc603](https://github.com/manustays/quay/commit/7fcc603c3101a528c5873392921711bc35af692d))
* **radar:** agent session discovery backend ([1134a4a](https://github.com/manustays/quay/commit/1134a4a8cf37cadba5b296f7b03b55323c8f0f4a))
* **radar:** detect Tauri as primary tech-stack over frontend bundler ([598cc61](https://github.com/manustays/quay/commit/598cc61d8733854266fe2ac27f719d476c2c333a))
* **radar:** project identity + session names in agent discovery ([da156d1](https://github.com/manustays/quay/commit/da156d1a46e3e9c02c1c2140f296a49f761b88a6))
* **ui:** agents section in popup ([cc33f4b](https://github.com/manustays/quay/commit/cc33f4b9c3daa976c3163e5fc5720d62a8c40bd2))
* **ui:** align agent folder rows, move tech-stack icon to right edge ([eb94c51](https://github.com/manustays/quay/commit/eb94c510046fdf02aac52c31fe6a70f5872e5054))
* **ui:** claude icon, agent folder clubbing, agents above More ([d3cb22b](https://github.com/manustays/quay/commit/d3cb22bddf1ffa99ab91354b192f0569cf7ed7a2))
* **update:** in-app update banner with daily background check ([6bf7508](https://github.com/manustays/quay/commit/6bf7508243b426505a7d0334b111ff3773bb568b))

# [0.15.0](https://github.com/manustays/quay/compare/v0.14.0...v0.15.0) (2026-07-06)


### Bug Fixes

* only toggle autostart when state changes to stop repeated BTM notification ([94d7fd1](https://github.com/manustays/quay/commit/94d7fd19bb64e03cffb567fea5808b24505fde54))


### Features

* add command service kind for CLI-managed detached daemons ([9438dc2](https://github.com/manustays/quay/commit/9438dc28261d3f754587dece6b37dc8a1de998eb))
* update assets for improved visuals and remove outdated images ([2725da2](https://github.com/manustays/quay/commit/2725da2d864621f3f97631cf235469c7d08a4c3d))

# [0.14.0](https://github.com/manustays/quay/compare/v0.13.1...v0.14.0) (2026-07-05)


### Bug Fixes

* adjust padding for improved layout in DetectedRow and Popup components ([64fd12e](https://github.com/manustays/quay/commit/64fd12e177f7123016a58b4dd75a6b3cfd640ab9))


### Features

* richer port radar — manifest names, more stacks, dev-only filter ([9bec927](https://github.com/manustays/quay/commit/9bec927c863249670ba65da3b78f4da7756589d7))

## [0.13.1](https://github.com/manustays/quay/compare/v0.13.0...v0.13.1) (2026-07-04)


### Bug Fixes

* prevent tray halo clipping at top edge ([356a0bc](https://github.com/manustays/quay/commit/356a0bcc6001e2690343cc6911bb5edd52ba69f8))

# [0.13.0](https://github.com/manustays/quay/compare/v0.12.0...v0.13.0) (2026-07-04)


### Features

* add funding configuration and social media preview image ([1f39c1e](https://github.com/manustays/quay/commit/1f39c1e81be9b7623c5bf42fe1e331d4d4a58669))

# [0.12.0](https://github.com/manustays/quay/compare/v0.11.0...v0.12.0) (2026-07-03)


### Bug Fixes

* update README formatting for improved readability and consistency ([911be09](https://github.com/manustays/quay/commit/911be09bb0fa42b4475bf50cd849d4629f82add9))


### Features

* add new images for quay desktop in assets ([9f592e6](https://github.com/manustays/quay/commit/9f592e64f985f5d6a767d24ef28aa690042a0a2c))

# [0.11.0](https://github.com/manustays/quay/compare/v0.10.0...v0.11.0) (2026-07-03)


### Features

* implement dynamic resizing of popover based on content height ([aef5d00](https://github.com/manustays/quay/commit/aef5d00976510da3342be2a37e91ec24016757dd))
* implement search functionality in popup and enhance service row expansion handling ([d3fa3d0](https://github.com/manustays/quay/commit/d3fa3d012ad786279232fc800117ce5d281bd09a))
* update README with new images and layout enhancements ([aea9967](https://github.com/manustays/quay/commit/aea996757f2cd32c27ff17f5d1695343d1c71aa3))

# [0.10.0](https://github.com/manustays/quay/compare/v0.9.0...v0.10.0) (2026-07-03)


### Bug Fixes

* open terminal CLI tools without a folder ($HOME fallback) ([c2a1ca2](https://github.com/manustays/quay/commit/c2a1ca21b683d0d95b85fc820d088895f85e2612))


### Features

* declutter popover rows and glassier shell ([8295f0f](https://github.com/manustays/quay/commit/8295f0f4c13239f20bfb91b866d2e1a7e227e839))
* enhance group status handling with 'partial' state for mixed item statuses ([08de205](https://github.com/manustays/quay/commit/08de2057eec88c1422cbec666a19955aab2023e4))
* reset crashed services + row-action cleanup ([17825c3](https://github.com/manustays/quay/commit/17825c3f05c00c65ef3b65c2e69cc9470b291d60))

# [0.9.0](https://github.com/manustays/quay/compare/v0.8.0...v0.9.0) (2026-07-02)


### Bug Fixes

* cluster groups in Favorites too; collapsible Detected section ([86905fe](https://github.com/manustays/quay/commit/86905fe84ee797d88f129e986a8abbb7fe5b0641))


### Features

* copy URL, uptime, reveal in Finder, richer exit errors ([85d0996](https://github.com/manustays/quay/commit/85d099657562230c21cc5e64d7145e93a6138f13))
* group renders as collapsible row with aggregate metrics ([8dab5e9](https://github.com/manustays/quay/commit/8dab5e93e169095f46f7915293308de125f2f63c))
* port radar — discover, adopt, kill unmanaged listeners ([3ad21a5](https://github.com/manustays/quay/commit/3ad21a5e1adf07093a0cc7853daa48688fe8e03c))
* service groups — cluster related services, start/stop together ([3c6535e](https://github.com/manustays/quay/commit/3c6535ec041ca42b59399293d0b4078d0c5c196f))
* tech-stack detection with brand icons on service rows ([4f2e53d](https://github.com/manustays/quay/commit/4f2e53d5eaf026ac5292f45f178e52769c56e2b6))

# [0.8.0](https://github.com/manustays/quay/compare/v0.7.0...v0.8.0) (2026-07-02)


### Bug Fixes

* align search icon to input field in popup ([39b71ca](https://github.com/manustays/quay/commit/39b71ca737aebb9f272205aadbe1f4412ad0c72b))


### Features

* add app name and version to menubar context menu ([5db3be6](https://github.com/manustays/quay/commit/5db3be6b9d9102a239e72017cbd275f13b780b0a))
* glow tray beacon dot red/amber on error/starting states ([23d318a](https://github.com/manustays/quay/commit/23d318a890fb3ec7466bfb03ad97a487a52af070))
* implement drag-and-drop reordering for service rows in popup ([0153580](https://github.com/manustays/quay/commit/0153580a32277b5f44e74089d864692f6fc6cf0d))

# [0.7.0](https://github.com/manustays/quay/compare/v0.6.0...v0.7.0) (2026-06-29)


### Features

* add in-app auto-update via tauri-plugin-updater ([9a99903](https://github.com/manustays/quay/commit/9a999039e8e6447bab10381c4ab99e653750c198))

# [0.6.0](https://github.com/manustays/quay/compare/v0.5.2...v0.6.0) (2026-06-29)


### Features

* fully automated releases via semantic-release ([29a7e8a](https://github.com/manustays/quay/commit/29a7e8a3e2a16c4d4198c2ce4acfd001a1929323))
