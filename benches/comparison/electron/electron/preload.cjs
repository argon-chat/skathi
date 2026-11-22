const { contextBridge, ipcRenderer } = require("electron");

contextBridge.exposeInMainWorld("native", {
  add: (a, b) => ipcRenderer.invoke("add", a, b),
  len: (bytes) => ipcRenderer.invoke("len", bytes),
  bench: () => ipcRenderer.invoke("bench"),
});
