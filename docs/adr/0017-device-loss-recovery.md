# Device loss is signal-classified and recovered with bounded attempts

A lost wgpu/DX12 device is a platform event, not an application failure. The host keeps the runtime, its activations and their presentation snapshots, rebuilds one device/queue/renderer and one surface per live window, and treats recovery as complete only after the new device has presented a frame. A driver removal and an API misuse stay distinct: `Validation` is loss only while the current device's loss signal is set; without that signal it remains a fatal surface error. Recovery is bounded, never sleeps on the UI thread, and reports an unrecoverable loss as a typed `HostError` with the attempt count and the adapter used by the final attempt.

The single-screen hardware spike in this ticket did not observe a real device loss. The policy below is derived from wgpu's contract, the existing one-shot rebuild/preservation path, synthetic destruction tests and the spike's negative result; the current host does not yet implement the signal, bound, latch or typed error, and #211–#213 add them. This ADR makes no real-GPU recovery claim; real hardware evidence remains with #214. Raw spike readings are in [the device-loss spike record](../实现/证据/device-loss/spike.txt).

## Decision

### Classification

- Every device the host creates gets `set_device_lost_callback`. The callback is tagged with the device generation, records the reason and message in a shared loss signal, and posts the reserved `host_message::DEVICE_LOSS` message (`WM_APP + 2`) to a live window, or to the tray HWND when no window is open. Repeated callbacks and messages coalesce into at most one pending rebuild; the callback never touches the runtime, renderer, surfaces or scene and never rebuilds on a wgpu thread.
- A callback whose generation is not the current one is ignored. A callback from an old device cannot schedule a second rebuild or clear the new signal.
- `CurrentSurfaceTexture::Lost` marks loss directly, whether or not the callback has run yet. `CurrentSurfaceTexture::Outdated` reconfigures the surface once; if it stays outdated, that is a fatal surface error.
- `CurrentSurfaceTexture::Validation` is classified by the current generation's signal:
  - signal set: loss, because a removed device can cascade validation errors from surfaces that no longer have a device;
  - signal clear: fatal API misuse, reported as `HostError::Surface`, never rebuilt.
- `Timeout` and `Occluded` are not loss and do not rebuild. They retain damage for the next paint; hidden or occluded windows must not spin.
- A renderer error is a loss symptom: it sets the signal, and the rebuild that follows is an attempt. While the loss signal is set it is never a terminal render error; a renderer error during a rebuild or proving pass fails that attempt. A renderer failure that is proven unrelated to loss is outside this policy.
- The loss signal stays set across failed attempts. It is cleared only after the rebuilt device presents successfully, never by the rebuild alone.

### Recovery

- One recovery is in flight at a time. A loss callback or message that arrives while `recovering` only keeps the pending signal set; it cannot create a second device.
- One attempt stages a new adapter, device, queue, renderer and one surface per live window from the same adapter and format, then presents. Nothing is published to the host until every live window's surface exists; only then does the host replace the device/queue/renderer/surface fields and drop the old surfaces. A failure discards the staged objects and leaves the loss signal set. The lost generation is never used to create a surface or present a frame, and `ensure_surface` must not recreate a surface while loss is pending. The rebuild is all-or-nothing: a failure on a later window fails the attempt instead of leaving a mix of old and new devices.
- Recovery completes after the attempt's proving pass: at least one visible window acquired `Success`/`Suboptimal` and presented, and no visible window reported `Lost`, `Validation` or a renderer error. `Timeout` and `Occluded` are skipped, not success; they retain damage and leave the episode open. If no visible window can prove the device, the staged device is kept and no further rebuild happens until a visible window can attempt a present; if that present fails, the episode resumes with the remaining attempts. Waiting for a visible window is not a retry. The attempt counter resets only after a complete proving pass succeeds, never after an individual window's present within a multi-window pass. When the attempt kept its device and waited for a first present, that present completes the deferred pass; a loss reported afterwards by another window starts a new episode.
- The runtime is not recreated. Node IDs, focus, caret, selection, draft, appearance, visibility, window requests, scroll offset and the applied scene survive. After the new device exists the host rebuilds each scene from the runtime's latest presentation snapshot; the snapshot and Node IDs are not invalidated by the GPU loss.
- Hidden windows stay hidden and receive a surface but no present. Worker delivery and wake targeting are unaffected; a worker result delivered during recovery is applied by the normal runtime path and painted after the device is usable.
- Each failed attempt posts at most one pending-loss message and returns to the message loop. There is no UI-thread sleep, no repaint spin and no unbounded device creation.
- The bound is three attempts per loss episode. The counter starts when loss is signalled and resets only after a complete proving pass. Three is a small fixed policy that absorbs a driver-restart window and one one-shot failure without unbounded device creation; it is not a measured latency.
- After the third failed attempt the host latches one `HostError::DeviceLost(Box<DeviceLostError>)` and stops attempting. The proposed public shape, exported by `zenui-windows` with #213, is:
  ```text
  GpuAdapterInfo { name: String, backend: String, device_type: String,
                   driver: String, driver_info: String, fallback: bool }
  DeviceLostError { attempts: u32, adapter: GpuAdapterInfo, message: String }
  HostError::DeviceLost(Box<DeviceLostError>)
  ```
  `HostError` boxes this payload so the per-frame `Result`s that carry it stay small; the fields above are unchanged.
  `run` and `pump` return it; subsequent calls return the latched error without creating another device. The host does not panic, hang or silently continue on stale surfaces.
- Adapter selection during recovery is the same as initial creation: high-performance hardware first, the fallback adapter second. A successful recovery on WARP is allowed, but it is observable through `Host::adapter_info()` and its `fallback` flag; WARP is never a silent substitute for a hardware device after a loss.
- The proposed `Host::adapter_info()` returns the current `GpuAdapterInfo` for logs, acceptance records and applications that choose to show the active adapter. It is diagnostic, not a rendering-capability negotiation API. #213 adds it and the typed error.

### Evidence boundary

- Host unit tests that set `device_lost` directly (`device_loss_rebuilds_every_window_surface`, `device_loss_preserves_the_applied_scroll_offset`) and the temporary `Device::destroy()` probe in [spike.txt](../实现/证据/device-loss/spike.txt) are synthetic. They exercise the rebuild and preservation paths, not a driver removal.
- Real-hardware acceptance belongs to #214. The 2026-11-25 single-screen run attempted Win+Ctrl+Shift+B and a forced GPU hang; it observed a real `nvlddmkm` error event 153 but no TDR/reset or D3D12 device removal, so it claims no recovery. README and verification pages describe the attempt and this negative result; this ADR does not turn the synthetic coverage into a native claim. See [the device-loss attempt](../实现/stage-three-设备丢失-验收.md).

## Spike record

Date: 2026-11-25, 06:28:58–06:29:17 local. Windows 11 Pro 10.0.26300, one display at 2560×1600, 150% (144 DPI), NVIDIA GeForce RTX 4060 Laptop GPU, driver 32.0.15.7283, DX12, debug `todo_host`, app window 1372×1106 at (980,40). Raw readings: [spike.txt](../实现/证据/device-loss/spike.txt).

- Trigger: Win+Ctrl+Shift+B injected with `keybd_event` and explicit scan codes, twice in the persisted run (a tap and a 4 second hold). The same script verified Win-key injection with Win+D: the app minimized and restored. An earlier exploratory tap produced the same result but is not part of the persisted record.
- Observed: a sampled desktop pixel stayed at brightness 79 for both attempts; the app window stayed visible and enabled; task state was unchanged; the Reverse action still reordered the rows; the app stderr log contained the adapter, `WM_PAINT` and successful presents, but no device-lost callback, no `surface Lost`, no `surface Validation` and no rebuild. The System log check since 06:09:47 found zero `Display`/TDR events. No screen flash was observed.
- Not attempted: GPU disable/enable, driver restart and RDP transition. The session is not elevated and there is no second session to switch to; those triggers remain for #214 or a machine with administrator access.
- Synthetic companion: on the same GPU, `Device::destroy()` followed by `device.poll(wgpu::PollType::wait_indefinitely())` invoked the callback with `DeviceLostReason::Destroyed` and an empty message during the poll, not at `destroy`. This is a backend-specific synthetic observation (Vulkan in that probe; the host uses DX12), not a real removal.
- Conclusion: no real device loss was observed with this trigger and environment. No recovery is claimed from this run; #211–#213 implement the policy against synthetic tests, and #214 owns the real-hardware record.

## Considered options

- **Treat every Validation as loss**: hides API misuse and turns a programming error into an endless rebuild. Rejected; validation is loss only under the current loss signal.
- **Rebuild inside the wgpu callback**: executes host and renderer work on a wgpu thread, can re-enter the device while it is being torn down, and bypasses the UI thread's ownership. Rejected; the callback only sets the signal and posts a message.
- **Clear loss after a successful rebuild**: a rebuilt device that cannot present is not recovered. Rejected; the episode ends only after a complete proving pass.
- **Retry forever or sleep between attempts**: unbounded device creation or a blocked UI thread. Rejected; three message-driven attempts, then one typed error.
- **Fall back to WARP silently**: changes performance and capability without the application's knowledge. Rejected; WARP is reported through the adapter info and the terminal error.
- **Rebuild only the window that reported Lost**: all windows share one device and queue, so a per-window rebuild would mix generations. Rejected; the rebuild is all-or-nothing.
- **Keep the old device or surfaces after loss**: they are invalid; painting them produces stale output or further errors. Rejected.
- **Recreate the runtime or activation**: loses Node IDs, focus, caret, selection, draft and scroll, none of which the GPU failure invalidated. Rejected.

## Consequences

#211 implements the generation-guarded callback, the pending-loss message, the signal-gated `Validation` rule and the preservation path. #212 keeps damage until a successful present and prevents a spin while hidden. #213 adds the three-attempt bound, the latched `HostError::DeviceLost`, `Host::adapter_info()` and the WARP report. #214 recorded the single-screen real-hardware run on 2026-11-25; it observed a driver error event (`nvlddmkm` 153) but no TDR/reset or device loss, so the documentation says so and makes no recovery claim ([record](../实现/stage-three-设备丢失-验收.md)).

The SDK sentence at `docs/SDK/窗口与任务.md:320` that `Lost` / `Validation` both rebuild is stale today (the code already treats `Validation` as fatal) and is superseded by this classification; #211 corrects the SDK when the signal-gated rule lands. Multi-monitor and mixed-DPI device loss remain outside this ADR and stay with #63.
