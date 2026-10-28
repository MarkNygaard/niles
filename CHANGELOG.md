# Changelog

## [0.7.0](https://github.com/MarkNygaard/niles/compare/v0.6.0...v0.7.0) (2026-10-28)


### Features

* **add-ons:** no Custom badge on Niles's own add-ons, and a Devices group (NIL-57) ([#494](https://github.com/MarkNygaard/niles/issues/494)) ([b5edfb1](https://github.com/MarkNygaard/niles/commit/b5edfb12d2fdb08d6aead3296ebd6481d9eae5d8))
* **artifacts:** a new trip or training plan comes with its own page, the built-in view only a fallback (NIL-66) ([#506](https://github.com/MarkNygaard/niles/issues/506)) ([8de9a90](https://github.com/MarkNygaard/niles/commit/8de9a905dd16b5eb7bbbb6776fd213f897239700))


### Fixes

* **niles-plugins:** link speech-stream at 0.6.0 under its WIT name (NIL-62) ([#500](https://github.com/MarkNygaard/niles/issues/500)) ([14d0945](https://github.com/MarkNygaard/niles/commit/14d09452ecfca01467e97c1f8cc7a0a06ec155f4))
* **web:** the header stays at the top on a phone, as on a desk (NIL-63) ([#502](https://github.com/MarkNygaard/niles/issues/502)) ([2734887](https://github.com/MarkNygaard/niles/commit/2734887c54687381e9d03ea84d81678e4e5a1337))

## [0.6.0](https://github.com/MarkNygaard/niles/compare/v0.5.0...v0.6.0) (2026-10-24)


### Features

* **add-ons:** several add-ons in one repo, as owner/repo/name (NIL-49) ([#491](https://github.com/MarkNygaard/niles/issues/491)) ([394f4ef](https://github.com/MarkNygaard/niles/commit/394f4efa1f08dce49709b22855b68ef85c9f9189))


### Fixes

* **tts:** talk to the current Piper (/synthesize), and follow a changed Piper address without a restart (NIL-47) ([#487](https://github.com/MarkNygaard/niles/issues/487)) ([967ace6](https://github.com/MarkNygaard/niles/commit/967ace6fd9b7eeb941af46f49ac829fe695cafff))

## [0.5.0](https://github.com/MarkNygaard/niles/compare/v0.4.0...v0.5.0) (2026-10-22)


### Features

* **add-ons:** browse the add-on library, with install counts and no telemetry (NIL-35) ([#469](https://github.com/MarkNygaard/niles/issues/469)) ([d9514cc](https://github.com/MarkNygaard/niles/commit/d9514cc529680f6f13448a2d0e11fd7baef9a33a))
* **artifacts:** edit saved items by part, and a timed-out change becomes a job (NIL-37) ([#474](https://github.com/MarkNygaard/niles/issues/474)) ([bca213a](https://github.com/MarkNygaard/niles/commit/bca213aaae54c9d656fa2461acbccd565b02f09f))
* notifications page behind a bell; the dashboard is only the house (NIL-46) ([#483](https://github.com/MarkNygaard/niles/issues/483)) ([13f45dd](https://github.com/MarkNygaard/niles/commit/13f45dddae0f1cc729e36f3481bf7831fe146ef2))
* **plugins:** host-managed speech sessions (NIL-42) ([#482](https://github.com/MarkNygaard/niles/issues/482)) ([d5a47cc](https://github.com/MarkNygaard/niles/commit/d5a47cc93344baf9eb860a073481beffb44e7b7f))
* **plugins:** host-managed WebSocket support (NIL-36) ([#470](https://github.com/MarkNygaard/niles/issues/470)) ([e11a5c7](https://github.com/MarkNygaard/niles/commit/e11a5c7bf41e33ebe10379fb38866f724ee6f004))
* **plugins:** room-only announcements with tracked delivery outcomes (NIL-43) ([#484](https://github.com/MarkNygaard/niles/issues/484)) ([a8595d2](https://github.com/MarkNygaard/niles/commit/a8595d246447d616f092179bf08f1fecd9ec628d))
* push a notification when something waits for approval (NIL-16) ([#485](https://github.com/MarkNygaard/niles/issues/485)) ([254c23d](https://github.com/MarkNygaard/niles/commit/254c23dc4200e6996b9340789acfdb71d1f5b261))
* **site:** getniles.app: a landing page, the docs and the add-on library ([#475](https://github.com/MarkNygaard/niles/issues/475)) ([73e5147](https://github.com/MarkNygaard/niles/commit/73e5147bb3822ee66c905ada2af37b5767be0473))
* **web:** a share menu on an open artifact, with Word and Markdown downloads ([#472](https://github.com/MarkNygaard/niles/issues/472)) ([4160427](https://github.com/MarkNygaard/niles/commit/4160427a126e966c1f507d08447eb6983d77c65d))

## [0.4.0](https://github.com/MarkNygaard/niles/compare/v0.3.0...v0.4.0) (2026-10-17)


### Features

* **assistant:** Niles's assistant writes by the unslop rules ([#461](https://github.com/MarkNygaard/niles/issues/461)) ([4e719b7](https://github.com/MarkNygaard/niles/commit/4e719b7a5bcccd7c8df04d1308e635ed0f298571))
* **podcasts:** research a topic, make a two-host episode — Kokoro voices, an R2 add-on, Sonos playback (NIL-6) ([#457](https://github.com/MarkNygaard/niles/issues/457)) ([5364179](https://github.com/MarkNygaard/niles/commit/5364179565b7999b5ac8933668f338b044dc564b))


### Fixes

* **auth:** undo can't empty the allowlist, a warning when sign-in is off, and the model can't change who signs in ([#462](https://github.com/MarkNygaard/niles/issues/462)) ([f9c6458](https://github.com/MarkNygaard/niles/commit/f9c64581334c408dec4b6f503c637a317efcf5ed))
* lights keep manual mode, room scenes by name, house announcements, notification history and the Linear webhook ([#466](https://github.com/MarkNygaard/niles/issues/466)) ([042ef5d](https://github.com/MarkNygaard/niles/commit/042ef5de49490e02644e2bad216ff2bc71f6f8a6))
