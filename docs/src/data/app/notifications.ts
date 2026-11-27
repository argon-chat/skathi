import { OK, PART, importLine, os, section } from "./shared";

export const notifications = section("Notifications", [
  {
    slug: "app-notifications",
    name: "appNotifications",
    kind: "interface",
    since: "Unreleased",
    summary: "Post a native desktop notification, replace or withdraw it by ID, and handle the user's response.",
    platforms: os(OK, OK, PART),
    syntax: `${importLine("appNotifications")}

appNotifications.capabilities(): Promise<NotificationCapabilities>
appNotifications.show(options: DesktopNotificationOptions): Promise<NotificationReceipt>
appNotifications.close(id: string): Promise<void>
appNotifications.setActivationHandler(handler: ((event: NotificationActivation) => void) | null): void`,
    params: [
      {
        name: "options.title",
        type: "string",
        description: "Nonblank plain-text title. Supply localized text for the title, body and labels.",
      },
      { name: "options.body", type: "string", optional: true, description: "Plain-text detail below the title." },
      {
        name: "options.id",
        type: "string",
        optional: true,
        description:
          "A stable ID scoped to this app and profile. Reusing it replaces the whole card and may alert again. Without one, show() generates an ID.",
      },
      {
        name: "options.actions",
        type: "readonly NotificationAction[]",
        optional: true,
        description:
          'Up to two actions with unique IDs. Each has an id, a label and kind "button" or "reply". Only one reply is allowed.',
      },
      {
        name: "options.sound",
        type: '"default" | "silent"',
        optional: true,
        description:
          'Defaults to "default". Requesting "silent" on a backend without silent delivery rejects with NotSupportedError.',
      },
      {
        name: "options.group",
        type: "{ id: string; label: string }",
        optional: true,
        description: "Visually groups related cards where supported. Give each card its own notification ID.",
      },
      {
        name: "options.image",
        type: "{ asset: string } | { png: Uint8Array }",
        optional: true,
        description:
          "A packaged raster asset or PNG bytes in a non-shared Uint8Array. Input is limited to 2 MiB and 4096 pixels per side. The thumbnail fits within 512 pixels per side.",
      },
    ],
    notes: [
      "Use `capabilities()` to check `available`, `maxActions`, `inlineReply`, `silent`, `image` and `grouping` before presenting choices. Permission is checked separately through `appPermissions`. Calling `show()` never opens a permission prompt.",
      'The `management` capability describes how long IDs remain usable. With `"session"`, the current backend session can replace or close a card. With `"application"`, the same app and profile can also do so after a restart. Closing an absent ID succeeds.',
      'The `activation` capability is `"none"`, `"running"` or `"relaunch"`. Register the handler at startup to drain queued events in order. Each event includes the card ID. A button also carries `actionId`, and a reply adds `text`. Pass `null` to detach the handler.',
      "A successful `show()` returns `id`, `submittedActions` (the encoded action IDs in order) and `omitted` (requested body, group or image fields the backend left out). The OS controls whether a banner appears and which actions are visible.",
    ],
    examples: [
      {
        title: "Send a test alert from a settings button",
        lang: "ts",
        code: `import { appNotifications, appPermissions } from "skathi:app";

export function routeTestAlert(openSettings: () => void) {
  appNotifications.setActivationHandler((event) => {
    if (event.id === "settings-test" && event.kind === "open") openSettings();
  });
}

export async function sendTestAlert() {
  const caps = await appNotifications.capabilities();
  if (!caps.available) return;
  let state = appPermissions.query("notifications");
  if (state === "prompt") state = await appPermissions.request("notifications");
  if (state === "denied" || state === "prompt") return;

  return appNotifications.show({
    id: "settings-test",
    title: "Desktop alerts are ready",
    body: caps.activation === "none" ? "Your test alert arrived." : "Open this alert to return to settings.",
  });
}`,
        note: "Register the route when the app starts and call sendTestAlert() from a user click. On Wayland a usable service can post when the permission query returns unsupported.",
      },
    ],
    limitations: [
      "macOS requires an application bundle. An embedder that owns the notification delegate can post passive cards. A cold launch from a button or reply can show the initial window if macOS delivers the action after the first reveal.",
      "On Windows, the Skathi MSIX toast activator can relaunch the app from Notification Center. An unpackaged app can handle clicks on a visible banner while running.",
      "On Wayland, the XDG portal can deliver images and may advertise inline reply. The direct notification service has session-scoped IDs and may expose only plain text. Linux activation requires a running app. Application-defined visual grouping is unavailable.",
    ],
    edgeCases: [
      "`show()` rejects with `TypeError` for invalid input, `NotAllowedError` when posting lacks permission, `NotSupportedError` when the backend or requested silent mode is unavailable, and `OperationError` for a service failure.",
    ],
    related: ["app-permissions", "permissions-request"],
    aliases: [
      "desktop notification",
      "toast",
      "appNotifications.show",
      "appNotifications.capabilities",
      "appNotifications.close",
      "appNotifications.setActivationHandler",
    ],
  },
]);
