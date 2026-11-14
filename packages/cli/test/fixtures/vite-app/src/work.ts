// Never bundled: `?worker` and `new Worker(new URL(...))` become named NotSupportedError stubs.
self.onmessage = () => {};
