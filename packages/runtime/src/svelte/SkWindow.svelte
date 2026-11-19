<script lang="ts">
  import { untrack } from "svelte";
  import { createAttachmentKey, type Attachment } from "svelte/attachments";
  import { createSkWindowPopout, registerWindowRoot, type SkWindowTarget } from "@skathi-native/runtime";
  import type { SkWindowProps } from "../svelte";

  let {
    title,
    bounds,
    chrome,
    backdrop,
    alwaysOnTop,
    resizable,
    minWidth,
    minHeight,
    titleBarHeight,
    activate,
    visible,
    cover,
    taskbar,
    fullResolution,
    lowLatencySurface,
    ignoreMouseEvents,
    focusable,
    contentProtection,
    monitor,
    onopen,
    onclose,
    onfocus,
    onblur,
    onresize,
    onshow,
    onhide,
    el = $bindable(null),
    windowId = $bindable(0),
    width = $bindable(0),
    height = $bindable(0),
    scale = $bindable(1),
    class: className,
    children,
    ...rest
  }: SkWindowProps = $props();

  const popout = createSkWindowPopout(
    () => ({
      title,
      bounds,
      chrome,
      backdrop,
      alwaysOnTop,
      resizable,
      minWidth,
      minHeight,
      titleBarHeight,
      activate,
      visible,
      cover,
      taskbar,
      fullResolution,
      lowLatencySurface,
      ignoreMouseEvents,
      focusable,
      contentProtection,
      monitor,
    }),
    {
      open: (id) => onopen?.(id),
      close: () => onclose?.(),
      focus: () => onfocus?.(),
      blur: () => onblur?.(),
      resize: (metrics) => onresize?.(metrics),
      show: (shown) => onshow?.(shown),
      hide: () => onhide?.(),
    },
    {
      get windowId() {
        return windowId ?? 0;
      },
      set windowId(v) {
        windowId = v;
      },
      get width() {
        return width ?? 0;
      },
      set width(v) {
        width = v;
      },
      get height() {
        return height ?? 0;
      },
      set height(v) {
        height = v;
      },
      get scale() {
        return scale ?? 1;
      },
      set scale(v) {
        scale = v;
      },
    },
  );

  export function mapFromMain(target: Element | null | undefined) {
    return popout.mapFromMain(target as SkWindowTarget | null | undefined);
  }

  $effect(() => {
    popout.options();
    untrack(popout.sync);
  });

  // Spread rather than `{@attach}`: the attachment is then made before the children's effects, so they see their
  // window. Svelte detaches the root before the teardown but does not free it, so the close still reaches the host.
  const root: Attachment<HTMLElement> = (node) => {
    const target = node as unknown as SkWindowTarget;
    const unregister = registerWindowRoot(target);
    el = node;
    untrack(() => popout.open(target));
    return () => {
      popout.close();
      unregister();
    };
  };
  const rootAttachment = { [createAttachmentKey()]: root };
</script>

<div {...rest} {...rootAttachment} class={["sk-window-root", className]}>
  {@render children?.()}
</div>
