# vTally Frontend

The web interface of the vTally hub, built with ReactJS (create-react-app). It is served by the
hub ([`../hub-rs`](../hub-rs)) and talks to it over socket.io. You only need Node.js to build it, not to run the hub.

The protocol types (`src/lib/SocketEvents.ts`) and the configuration classes (`src/mixer/*/*Configuration.ts`,
`src/tally`, `src/domain`) mirror what the hub sends. If the protocol changes, change both sides.

## Development

    npm ci
    npm run build      # production build in ./build
    npm run test:ci    # unit tests

Run the hub with the built frontend and open http://localhost:3000:

    cd ../hub-rs && cargo run -- --frontend-dir ../frontend/build

### Cypress

The end-to-end tests drive the frontend and a hub with the scripted test mixer:

    # build the frontend and start the hub with the test mixer
    npm run build
    (cd ../hub-rs && cargo run -- --with-test --frontend-dir ../frontend/build)
    # in another terminal
    npx cypress open
