import { mount } from "@skathi-native/runtime/svelte";
import App from "./App.svelte";

mount(App, { props: { heading: "Native renderer" }, context: new Map([["suffix", "!"]]) });
