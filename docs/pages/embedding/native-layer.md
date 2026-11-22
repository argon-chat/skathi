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

- **`attach`** is called when the element is painted in a window for the first time, when it moves to another window, and when the window rebuilt what it composes with (a lost device on Windows). A placement always follows it.
- **`place`** is called from the paint that draws the element, so the layer moves in the frame the page does, and only when the placement changed. `None` means the element was not painted in the last frame: `display: none`, scrolled out, or culled.
- Both run on Skathi's UI thread inside the page's paint. Return quickly and do not call into Skathi from them.

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

| Field         | Means                                                                                          |
| ------------- | ---------------------------------------------------------------------------------------------- |
| `window`      | the window's Skathi ID                                                                         |
| `handle`      | the raw window handle: the `HWND`, on macOS the `NSView` the page is drawn in                  |
| `composition` | Windows: the window's `IDCompositionDevice` and the `IDCompositionVisual` drawn under the page |

**Windows.** Skathi's window owns the DirectComposition target, so a target of your own is refused and a child window lands over the page. Create your visual on `composition.device` (a visual belongs to the device that made it) and add it to `composition.underlay`. Both pointers are borrowed for the call: clone the interfaces to keep them, and drop the old ones on the next `attach`. Call `Commit` on the device after you change the visual.

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

**macOS.** `handle` is `RawWindowHandle::AppKit`, whose `ns_view` is the view the page is drawn in. Add your view to its superview, below it (`addSubview:positioned:relativeTo:` with `NSWindowBelow`), so it stays inside the window's content view and moves with it.

**Linux.** `attach` and `place` are called, but there is no place under the page yet.

Next: [Window, tray and hotkeys](/embedding/desktop/).
