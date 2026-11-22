# vTally Frontend

The web interface of the vTally hub, built with ReactJS (create-react-app). It is served by the
hub ([`../hub-rs`](../hub-rs), or the deprecated Node.js hub in [`../hub`](../hub)) and talks to it
over socket.io.

The protocol types (`src/lib/SocketEvents.ts`) and the configuration classes (`src/mixer/*/*Configuration.ts`,
`src/tally`, `src/domain`) are shared with the hub by convention: if the protocol changes, change it in the hub, too.

## Development

    npm ci
    npm start          # dev server on port 3001

Start a hub that proxies to it, e.g. the Node.js hub (`npm run start:backend` in `../hub`), and open
http://localhost:3000. The Rust hub does not proxy: build the frontend and pass `--frontend-dir build`.

## Build and test

    npm run build      # production build in ./build
    npm run test:ci    # unit tests

### Cypress

The end-to-end tests drive the frontend and a hub with the scripted test mixer:

    # in ../hub:      npm ci && npm run cypress:backend
    # in this folder: npm start, then
    npx cypress open
