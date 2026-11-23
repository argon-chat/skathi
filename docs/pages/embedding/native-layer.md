---
title: "A native layer under the page: <sk-layer>"
label: Native layer
description: Put a native layer of your own (a video player's swap chain, a Metal layer) under a Skathi element, told where the element is from the frame that paints it, and moved with it between windows.
section: Embedding
order: 6.5
---

[`<sk-surface>`](/embedding/native-frames/) takes frames and draws them as part of the page. A program that must present by itself, such as a player with its own swap chain and HDR output, wants the opposite: its own layer on screen, with the page over it. `<sk-layer>` is the element for that. It paints nothing; your layer is shown under the page where the element is.

```vue
<template>
  <sk-layer name="player" class="video" />
</template>
```

The page has to be transparent there to let the layer show: a window with [`backdrop: "transparent"`](/embedding/desktop/#a-backdrop-the-page-cannot-change) and no background over the element.

## The layer

Register a layer by name in `setup`. Each element of that name gets one of its own, made when the element is first painted and dropped when the element or its window goes.

```rust
use skathi::{Layer, LayerHost, LayerPlacement};

struct Player { /* your visual, swap chain, view */ }

impl Layer for Player {
    fn attach(&mut self, host: &LayerHost) {
        // Move the layer into this window; see below.
    }

    fn place(&mut self, placement: Option<LayerPlacement>) {
        match placement {
            Some(at) => { /* show at at.bounds, clipped to at.clip */ }
            None => { /* hide */ }
        }
    }
}

let builder = skathi::Builder::new().setup(|host| {
    host.layer("player", || Box::new(Player::new()));
    Ok(())
});
```

- **`attach`** is called when the element is painted in a window for the first time, when it moves to another window, when the window's view is moved into another native window or under another superview (macOS), and when the window rebuilt what it composes with (a lost device on Windows). A placement always follows it.
- **`place`** is called from the paint that draws the element, before that frame is presented, and only when the placement changed. `None` means the element was not painted in the last frame: `display: none`, scrolled out, or culled.
- **`presented`** (optional, Windows) is called once Skathi has committed your changes with the frame that carried them; see below.
- All run on Skathi's UI thread inside the page's paint or present. Return quickly and do not call into Skathi from them.

`LayerPlacement` is in device pixels of the window:

| Field     | Means                                            |
| --------- | ------------------------------------------------ |
| `bounds`  | the element's box: `[x, y, width, height]`       |
| `clip`    | the part of it inside the clips of its ancestors |
| `radii`   | its corner radii, top-left clockwise             |
| `scale`   | the window's scale factor                        |
| `opacity` | the element's own `opacity`                      |

A CSS transform on the element or an ancestor is not applied to the placement.

## Where the layer goes

`LayerHost` says where, per platform:

| Field           | Means                                                                                          |
| --------------- | ---------------------------------------------------------------------------------------------- |
| `window`        | the window's Skathi ID                                                                         |
| `handle`        | the raw window handle: the `HWND`, on macOS the `NSView` the page is drawn in                  |
| `composition`   | Windows: the window's `IDCompositionDevice` and the `IDCompositionVisual` drawn under the page |
| `native_window` | macOS: the `NSWindow` the page's view is in now                                                |

**Windows.** Skathi's window owns the DirectComposition target, so a target of your own is refused and a child window lands over the page. Create your visual on `composition.device` (a visual belongs to the device that made it) and add it to `composition.underlay`. Both pointers are borrowed for the call: clone the interfaces to keep them, and drop the old ones on the next `attach`.

```rust
fn attach(&mut self, host: &LayerHost) {
    let Some(composition) = host.composition else {
        return; // no DirectComposition in this window: nothing can be shown under it
    };
    let (device, underlay) = unsafe {
        (
            IDCompositionDevice::from_raw_borrowed(&composition.device.as_ptr()).unwrap().clone(),
            IDCompositionVisual::from_raw_borrowed(&composition.underlay.as_ptr()).unwrap().clone(),
        )
    };
    // create the visual on `device`, set its content, underlay.AddVisual(&visual, true, None)
}
```

`composition` is `None` when the window does not present through DirectComposition; that is the answer to whether a layer can be shown under this window at all.

**Do not call `Commit` from `attach` or `place`.** The device is shared, and Skathi commits it right after it presents the frame those calls came from, so your visual's changes and the page's frame go out together, and a frame that is dropped (during a live resize) does not leave the layer ahead of the page. A hide is committed at once, since the frame without the element is already on screen. `presented` tells you when that commit was made. A `Commit` of your own still works, but it reaches the compositor before the page's frame does.

What Skathi commits is the visual: its offset, clip and transform. If the layer's content must be redrawn at a new size, as a player's swap chain is after a resize, that picture reaches the screen when your renderer presents it. To keep it in the page's frame, present it before `place` returns, or scale the last picture with the visual's transform until your renderer catches up.

**macOS.** `handle` is `RawWindowHandle::AppKit`, whose `ns_view` is the view the page is drawn in. Add your view to its superview, below it (`addSubview:positioned:relativeTo:` with `NSWindowBelow`), so it stays inside the window's content view and moves with it.

When your program moves that view, or the content view holding it, into a window of its own, the page is redrawn there and `attach` is called again with the same `window` and `handle` and the new `native_window`, then `place`. Read the view's `superview` again then: a view of yours that moved with the content view needs nothing more; one left behind belongs under the view's new superview.

### A window Skathi did not create

While the page's view is in a window of yours, Skathi's window follows the view there:

- **Goes to your window:** what the page asks of its window (title, size and position, minimize, zoom, fullscreen, always on top, opacity, size limits, the backdrop's window colour), and what Skathi reads from it (bounds, screen and scale, appearance, key status, occlusion, the mouse position).
- **Heard from your window by notification:** moves, screen changes, occlusion, becoming and resigning key, and entering and leaving fullscreen. Skathi does not replace your window's delegate.
- **Stays with Skathi's own window:** its delegate, window tabs, restorable state, the page's own request to close, and Skathi's titlebar chrome (button positions, titlebar transparency, the fullscreen desktop picture), which your window keeps as you set it.
- **Not handled:** files dragged onto your window, which reach the page only in Skathi's own window.

Skathi's own window keeps no part in the page meanwhile: its delegate is ignored, and closing it closes only that empty window. Skathi retains your window while the view is in it. Move the view back, or remove it, before Skathi closes its window: the page ends with that window.

**Linux.** `attach` and `place` are called, but there is no place under the page yet.

Next: [Window, tray and hotkeys](/embedding/desktop/).
