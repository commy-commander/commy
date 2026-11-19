<a id="readme-top"></a>

<div align="center">

<a href="https://commy.chat">
  <img src="auth/pages/src/logo.svg" alt="Commy logo" width="110" height="110">
</a>

<h1>Commy</h1>

<p>
  <strong>A home for your community.</strong><br />
  Free, open source voice, video and text chat, built in Rust.
</p>

[![Download][download-shield]][download-url]
[![Support Commy on Ko-fi][kofi-shield]][kofi-url]
[![License: AGPL-3.0][license-shield]][license-url]

<p>
  <a href="https://commy.chat"><strong>commy.chat</strong></a>
  &middot;
  <a href="https://github.com/commy-commander/commy/issues/new?template=bug_report.yml">Report a bug</a>
  &middot;
  <a href="https://github.com/commy-commander/commy/issues/new?template=feature_request.yml">Request a feature</a>
</p>

</div>

<details>
  <summary>Table of contents</summary>
  <ol>
    <li><a href="#about-the-project">About the project</a></li>
    <li><a href="#getting-started">Getting started</a></li>
    <li><a href="#learn-more">Learn more</a></li>
    <li><a href="#contributing">Contributing</a></li>
    <li><a href="#support-commy">Support Commy</a></li>
    <li><a href="#license">License</a></li>
    <li><a href="#contact">Contact</a></li>
    <li><a href="#acknowledgments">Acknowledgments</a></li>
  </ol>
</details>

## About the project

Commy is where your people hang out: text channels for the everyday chatter,
voice channels to drop into, and screen sharing for game night. It is free to
use, and the whole app is open source.

- 💬 **Chat that feels alive.** Replies, reactions, threads, forums, polls,
  custom emoji and voice messages.
- 🎙️ **Voice that just works.** One click noise cancellation, a volume for
  each person, a soundboard, and stage channels for big moments.
- 🎮 **Show your game.** Share a whole screen or a single window, with sound
  on Windows.
- 🛡️ **Tools for moderators.** Roles and permissions, auto moderation,
  timeouts, bans and an audit log.
- 🔒 **Private by design.** Messages sync sealed end to end, and voice, video,
  screen shares and files go straight between members.
- 🏠 **Yours to host.** Run your own sync host and keep your community's
  history on your own server.

[See everything Commy can do →](docs/features.md)

### Built with

[![Rust][rust-shield]][rust-url]
[![Tauri][tauri-shield]][tauri-url]
[![WebAssembly][wasm-shield]][wasm-url]
[![Node.js][node-shield]][node-url]
[![PostgreSQL][postgres-shield]][postgres-url]

<p align="right">(<a href="#readme-top">back to top</a>)</p>

## Getting started

**Just want to chat?** Grab Commy for Windows, macOS or Linux at
[commy.chat/download](https://commy.chat/download), make an account, and start
a server or join one with an invite link.

**Want to build it?** You need Rust (stable), Node.js and the Tauri 2 CLI.

```sh
git clone https://git.commy.chat/commy/commy.git
cd commy/apps/desktop/src-tauri
cargo tauri dev
```

The [build guide](docs/building.md) covers setup on each platform, the web
client and the phone apps.

<p align="right">(<a href="#readme-top">back to top</a>)</p>

## Learn more

- [Everything Commy can do](docs/features.md)
- [How Commy works](docs/how-it-works.md)
- [Building Commy](docs/building.md)
- [Verifying a download](docs/verifying-downloads.md)
- [Hosting your own server](deploy/HOSTING.md)
- [Running bots](crates/commy-bot/README.md)

<p align="right">(<a href="#readme-top">back to top</a>)</p>

## Contributing

Ideas, fixes and translations are all welcome. Start with
[CONTRIBUTING.md](CONTRIBUTING.md) and the [Code of Conduct](CODE_OF_CONDUCT.md),
then open a pull request into `main`. Found a problem? Open an
[issue](https://github.com/commy-commander/commy/issues/new/choose). For
security problems, follow [SECURITY.md](SECURITY.md) instead.

<p align="right">(<a href="#readme-top">back to top</a>)</p>

## Support Commy

Commy is free, and it stays that way thanks to people like you. If Commy is
home for your community, you can help keep the lights on with a tip on
[Ko-fi](https://ko-fi.com/commychat). ☕

[![Support Commy on Ko-fi][kofi-shield]][kofi-url]

<p align="right">(<a href="#readme-top">back to top</a>)</p>

## License

Commy is licensed under the [GNU Affero General Public License v3.0](LICENSE).

## Contact

Commy Commander · [commy.chat](https://commy.chat) ·
[support@commy.chat](mailto:support@commy.chat)

## Acknowledgments

Commy stands on the shoulders of [Tauri](https://tauri.app),
[Opus](https://opus-codec.org), [nnnoiseless](https://github.com/jneem/nnnoiseless),
[Better Auth](https://www.better-auth.com) and
[Sigstore cosign](https://github.com/sigstore/cosign). README layout from
[Best-README-Template](https://github.com/othneildrew/Best-README-Template).

<p align="right">(<a href="#readme-top">back to top</a>)</p>

[download-shield]: https://img.shields.io/badge/download-commy.chat-3355ff.svg?style=for-the-badge
[download-url]: https://commy.chat/download
[kofi-shield]: https://img.shields.io/badge/Ko--fi-support%20Commy-FF5E5B.svg?style=for-the-badge&logo=ko-fi&logoColor=white
[kofi-url]: https://ko-fi.com/commychat
[license-shield]: https://img.shields.io/badge/license-AGPL--3.0-blue.svg?style=for-the-badge
[license-url]: LICENSE
[rust-shield]: https://img.shields.io/badge/Rust-000000?style=for-the-badge&logo=rust&logoColor=white
[rust-url]: https://www.rust-lang.org
[tauri-shield]: https://img.shields.io/badge/Tauri_2-24C8D8?style=for-the-badge&logo=tauri&logoColor=white
[tauri-url]: https://tauri.app
[wasm-shield]: https://img.shields.io/badge/WebAssembly-654FF0?style=for-the-badge&logo=webassembly&logoColor=white
[wasm-url]: https://webassembly.org
[node-shield]: https://img.shields.io/badge/Node.js-339933?style=for-the-badge&logo=nodedotjs&logoColor=white
[node-url]: https://nodejs.org
[postgres-shield]: https://img.shields.io/badge/PostgreSQL-4169E1?style=for-the-badge&logo=postgresql&logoColor=white
[postgres-url]: https://www.postgresql.org
