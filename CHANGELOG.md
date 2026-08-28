# Changelog

## [0.1.0-beta.2](https://github.com/slightlee/nocterm/compare/v0.1.0-beta.1...v0.1.0-beta.2) (2026-08-28)


### Bug Fixes

* allow verification of draft release assets ([#6](https://github.com/slightlee/nocterm/issues/6)) ([e5ba828](https://github.com/slightlee/nocterm/commit/e5ba82803e453ab479d4ac245f4abc5a0a60db74)), closes [#4](https://github.com/slightlee/nocterm/issues/4)
* build macOS installers for both architectures ([#5](https://github.com/slightlee/nocterm/issues/5)) ([f86e467](https://github.com/slightlee/nocterm/commit/f86e46756b8b69264a73d303c17e32e052627035)), closes [#4](https://github.com/slightlee/nocterm/issues/4)
* bundle desktop application icons ([#14](https://github.com/slightlee/nocterm/issues/14)) ([7fca53b](https://github.com/slightlee/nocterm/commit/7fca53b6a628ad967fd0786d106d910b99ef84d3)), closes [#12](https://github.com/slightlee/nocterm/issues/12)
* hide console window in Windows release builds ([eabdd3d](https://github.com/slightlee/nocterm/commit/eabdd3d922b71b1fdb89ce4be8e467e338ad502a))
* localize Windows installer ([#16](https://github.com/slightlee/nocterm/issues/16)) ([be71c5c](https://github.com/slightlee/nocterm/commit/be71c5c6dda96272bec4fda74ef029b968ca6c8e)), closes [#15](https://github.com/slightlee/nocterm/issues/15)
* prevent controls from triggering window drag ([e1b4a20](https://github.com/slightlee/nocterm/commit/e1b4a2006ed5f726620f13ce9d9f3a65f912dd8c)), closes [#9](https://github.com/slightlee/nocterm/issues/9)

## [0.1.0-beta.1](https://github.com/slightlee/nocterm/compare/b9a9270161c7fce3d33d0ef72ba678fe8899223c...v0.1.0-beta.1) (2026-08-27)


### Features

* adapt Windows window layout ([74a22ba](https://github.com/slightlee/nocterm/commit/74a22ba591ee97eab29a3259b27b150f3b94a7a5))
* add bulk close actions ([8ce9445](https://github.com/slightlee/nocterm/commit/8ce944553c71f3fa727dddf2ced61cf10f58c71e))
* implement connection and terminal capabilities ([4a7dbc4](https://github.com/slightlee/nocterm/commit/4a7dbc4d5fe385abd999b19d708817c2c282c042))
* implement SFTP file management ([6e10a80](https://github.com/slightlee/nocterm/commit/6e10a806e077565d05ed08512281a9de228f1ec7))
* migrate SSH and SFTP to in-process russh ([89b9461](https://github.com/slightlee/nocterm/commit/89b94617e0797c11f35818d7f6459d257857ec96))
* prefer PowerShell for Windows local terminals ([9a9ed55](https://github.com/slightlee/nocterm/commit/9a9ed55b052210ff4e9eabdb40faed4d57b74eb6))
* support copying terminal selections ([eb536aa](https://github.com/slightlee/nocterm/commit/eb536aaf802077f8a1499ba64a70a125e5bb8cec))


### Bug Fixes

* add Windows application icon ([62ca7ff](https://github.com/slightlee/nocterm/commit/62ca7ff58a4308b2fe3dd7c1a193d86550052656))
* align connection status indicators ([9928b96](https://github.com/slightlee/nocterm/commit/9928b9682ce8c29862968d1ba0e18aa97234ffce))
* enable dragging from tab bar gaps ([0029fb0](https://github.com/slightlee/nocterm/commit/0029fb078139609e7f9b42743227e245a2f75afb))
* make connection path test cross-platform ([2ed13a2](https://github.com/slightlee/nocterm/commit/2ed13a2720d2a05936c5a71abd3664d8447fa9fe))
* normalize cross-platform line endings ([7466350](https://github.com/slightlee/nocterm/commit/746635059bbfd94d100d5d58ec35983d92769ccd))
* restore native Windows title bar ([7b572b2](https://github.com/slightlee/nocterm/commit/7b572b28bf8c59666154b1f3f16f3e877a687d02))
* satisfy Windows terminal lint ([14fb62c](https://github.com/slightlee/nocterm/commit/14fb62c9e85d1f66188c77a06662546a30125569))
* validate release pull request head ([a7eedf6](https://github.com/slightlee/nocterm/commit/a7eedf6b4d3fa9b430cb0873a665fec59b699bda))
