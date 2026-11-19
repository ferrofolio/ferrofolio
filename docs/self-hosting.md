# Setup guide

Ferrofolio runs in Docker on a computer you control. This guide covers installing, reaching it from other devices,
updating, backups and the optional settings.

> [!WARNING]
> **Ferrofolio is not production ready.** The project is currently under heavy rewrite and is not reliable yet.
> Expect bugs and updates that break things, and keep [backups](#your-data-and-backups) of your data.

## What you need

- A computer that stays switched on, such as a home server, a NAS or a small PC, with
  [Docker](https://docs.docker.com/get-started/get-docker/) and Docker Compose.
- [Git](https://git-scm.com/downloads) to download the code.

Ferrofolio does not publish ready-made images yet. The example [docker-compose.yml](../docker-compose.yml) builds them
from this repository.

## Install

```sh
git clone https://github.com/ferrofolio/ferrofolio.git
cd ferrofolio
docker compose up -d --build
```

This starts two containers:

| Container | Port | Content |
|---|---|---|
| `ferrofolio-server` | 7592 | The server, which talks to your banks and stores your data |
| `ferrofolio-front` | 8080 | The web app you open in the browser |

Open <http://localhost:8080> and create your account. The encryption key you choose encrypts your data and cannot be
recovered, so keep it somewhere safe.

Because the data is encrypted with your key, the server is locked after every restart until you sign in again.
Automatic updates of your accounts only run while it is unlocked; to unlock it on start, set `USERNAME` and `PASSWORD`
(see [Settings](#settings)).

The web app works in any modern browser, on the desktop and on the phone. There is no desktop app. Offline use and
installing the web app on the home screen are in progress, and native mobile apps are planned in their own
repositories.

## Use it from other devices

The web app connects to the server at the address in `FINANZE_BASE_URL`, which is `http://localhost:7592` in the
example compose file. That address only works on the computer that runs Ferrofolio. To use it from your phone or
another computer in the same network:

1. Find the address of the computer that runs Ferrofolio in your network, for example `192.168.1.20`.
2. In `docker-compose.yml`, under `ferrofolio-front`, set `FINANZE_BASE_URL=http://192.168.1.20:7592`.
3. Run `docker compose up -d` again.
4. Open `http://192.168.1.20:8080` on the other device.

> [!WARNING]
> Do not make Ferrofolio reachable from the internet. The server is not hardened for that yet (see the
> [roadmap](roadmap.md), section 4.4). To use it away from home, connect to your home network through a VPN.

## Update

In the folder of your clone:

```sh
git pull
docker compose up -d --build
```

## Your data and backups

The server keeps all its data in the `.storage` folder next to `docker-compose.yml`. The database in it is encrypted
with your key. To back it up, stop Ferrofolio with `docker compose stop`, copy the `.storage` folder somewhere safe and
start it again with `docker compose start`.

The cloud sign-in and cloud backup of the app run on upstream Finanze services and do not work in Ferrofolio.

## Upgrade from Finanze

The upstream compose file named its services `finanze-server` and `finanze-front`, and their containers keep ports
7592 and 8080. Remove them before starting the renamed services:

1. Before updating, run `docker compose down` in the directory of the old setup. Compose names a project after the
   directory of its compose file, so the next step does not reach a setup in another directory.
2. In the directory of this repository, start the renamed services and remove containers left from the old ones:

   ```sh
   docker compose up -d --build --remove-orphans
   ```

Run the upgrade in the directory that holds the existing `./.storage` folder, or copy that folder into the new
checkout first: a new checkout starts with an empty `./.storage`. The old data is not deleted.

Images built from this repository are not configured for the upstream cloud sign-in and cloud backup, so these
features stop working after the upgrade. The local data in `./.storage` is not affected.

## Optional services

### Mintos (Selenium)

Mintos needs Selenium and related tools such as ffmpeg to solve its reCAPTCHA. The example compose file builds the
server image without them. To include them:

1. In `docker-compose.yml`, set the build argument `SELENIUM_SUPPORT` to `"true"`.
2. Uncomment the `selenium` service and set `WEBDRIVER_ADDRESS: http://selenium:4444` on `ferrofolio-server`.
3. Rebuild with `docker compose up -d --build`.

### Local AI models

Ferrofolio can use AI models that run on your own hardware, so no data leaves it. See
[AI providers](features.md#ai-providers) for the options. The example compose file contains an optional
`ferrofolio-llama` service that runs [llama.cpp](https://github.com/ggml-org/llama.cpp):

1. Uncomment the `ferrofolio-llama` service.
2. Put a long random value in a `.env` file next to `docker-compose.yml`: `FINANZE_LLAMA_API_KEY=...`.
3. Uncomment `FINANZE_LLAMA_SERVER_URL` and `FINANZE_LLAMA_SERVER_API_KEY` on `ferrofolio-server`.
4. Run `docker compose up -d`.

GGUF model files, together with their `mmproj` files, go into `./.storage/models/local`; press Refresh in the app to
list them. Models with a vision projector (`mmproj`) can read images and scanned PDFs. The comments in the compose file
cover GPU images and an external llama.cpp server.

An optional `ollama` service is included as well; connect it as a custom AI provider with the URL
`http://ollama:11434/v1`.

## Settings

The server reads these environment variables. Set them under `environment:` of `ferrofolio-server` in
`docker-compose.yml`, or in a `.env` file loaded with `env_file:`.

| Variable | Purpose |
|---|---|
| `USERNAME`, `PASSWORD` | Sign in automatically when the server starts |
| `MULTI_USER` | Allow more than one user to sign up. Only recommended for local development |
| `FINANZE_PUBLIC_URL` | Address of the server as seen from your browser, see [below](#public-address) |
| `FINANZE_API_SESSION` | `"false"` disables per-client API sessions, so any client that reaches the server can use the API once it is unlocked. Not recommended |
| `FINANZE_API_SESSION_REMEMBER_DAYS` | Days until a "Remember me" session expires. By default it never expires |
| `UNICAJA_ABCK` | Needed to sign in to Unicaja, which uses Akamai bot protection |
| `WEBDRIVER_ADDRESS` | Address of the Selenium service, for Mintos |
| `FINANZE_LLAMA_SERVER_URL`, `FINANZE_LLAMA_SERVER_API_KEY` | Address and key of the `ferrofolio-llama` service. Values entered in the app take precedence |
| `FF_<NAME>` | Sets a feature flag, for example `FF_TR_V2_AUTH_API=OFF` for the previous Trade Republic sign-in. Only applies while `FEATURE_FLAG_URL` is unset. The upstream cloud (`FF_CLOUD`) is off by default |
| `FEATURE_FLAG_URL` | Remote source for feature flags. When set, all flags come from it and `FF_` variables are ignored. Not set by default, so no request leaves the server for it |
| `CRYPTO_DATASET_URL`, `METAL_RATES_URL`, `CLOUD_URL` | Sources for a crypto coin snapshot, historic metal prices and cloud backups. None is set by default, so the server does not contact upstream services for them |

### Public address

`FINANZE_PUBLIC_URL` is the address under which the server is reachable from your browser, for example
`https://finance.example.com`. Two integrations redirect your browser back to it:

- **Enable Banking** redirects to `<address>/api/v1/entities/external/callback` after you gave consent. That URL has to
  be listed as an allowed redirect URL in your Enable Banking application.
- **Google Sheets** redirects to `<address>/oauth/callback` after you authorized access. That URL has to be listed as an
  authorized redirect URI of your Google OAuth client. Google only accepts such a URI with HTTPS and a domain name
  under a public top-level domain, with `localhost` and loopback addresses as the only exception. A raw IP address such
  as `https://192.168.1.10` or a local-only name such as `https://finance.lan` is rejected even over HTTPS, so for
  Google Sheets the server has to be reachable over HTTPS under such a domain name even inside a trusted local network.
  The name only has to resolve inside that network.

Without this variable the address is taken from the request, including the `X-Forwarded-Proto` and
`X-Forwarded-Host` headers of a reverse proxy. The browser carries the authorization code back to the app, so serve
the server over HTTPS unless it only runs in a trusted local network; over plain HTTP that code is sent unencrypted.

## How the server works

The entry point of the server image is the Rust server `ferrofolio`. It serves the routes that have been ported to
Rust and starts the Python backend inside the container for all others; the options and environment variables stay
the same. See [rust-backend.md](rust-backend.md).
