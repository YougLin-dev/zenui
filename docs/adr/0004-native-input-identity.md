# Native input dispatches by stable node ID and edits in the runtime

Native hit-testing and focus/IME need an identity that is stable across frames, and native typing needs insert/backspace/selection at a caret. The target runtime exposes an opaque Node ID in presentation snapshots and owns a platform-neutral text-edit reducer; label-based dispatch remains a test helper. The Windows host maps native messages to these operations and does not maintain a second editor state.
