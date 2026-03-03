// Empty module stub — used to alias node-only packages (e.g. `canvas`) to a
// no-op in browser/Turbopack builds. PDF.js attempts to optionally require
// `canvas` for server-side rendering; this stub prevents a bundler error.
export default {};
