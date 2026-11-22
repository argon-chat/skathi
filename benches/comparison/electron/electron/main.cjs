const { app, BrowserWindow, ipcMain } = require("electron");
const path = require("node:path");

ipcMain.handle("add", (_e, a, b) => a + b);
ipcMain.handle("len", (_e, bytes) => bytes.length);
ipcMain.handle("bench", () => !!process.env.COMPARE_BENCH);

app.whenReady().then(() => {
  const win = new BrowserWindow({
    width: 760,
    height: 520,
    title: "Compare",
    show: false,
    webPreferences: { preload: path.join(__dirname, "preload.cjs") },
  });
  win.once("ready-to-show", () => win.showInactive());
  win.loadFile(path.join(__dirname, "..", "dist", "index.html"));
});

app.on("window-all-closed", () => app.quit());
