<script setup lang="ts">
// TIER 2 — form + state components. Every one of these is a reka-ui primitive
// with real state, so this is where the DOM surface starts to matter:
// useFormControl(`.closest("form")`), Presence, RovingFocus, Collection.
import { ref } from "vue";
import type { AcceptableValue, CheckboxCheckedState, StringOrNumber } from "reka-ui";
import Section from "./Section.vue";
import { Input } from "../components/ui/input";
import { Textarea } from "../components/ui/textarea";
import { Checkbox } from "../components/ui/checkbox";
import { Switch } from "../components/ui/switch";
import { Toggle } from "../components/ui/toggle";
import { RadioGroup, RadioGroupItem } from "../components/ui/radio-group";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "../components/ui/tabs";
import { Accordion, AccordionContent, AccordionItem, AccordionTrigger } from "../components/ui/accordion";
import { Label } from "../components/ui/label";

// EVERY ref below is annotated with what the component can WRITE BACK, not
// with what its initial value happens to be. `v-model` is two-way, and an
// unannotated `ref("pro")` is a `Ref<string>` that a RadioGroup is entitled to
// put `null` into the moment nothing is selected. The four annotations that
// widen are not decoration — they are the four places this file was quietly
// lying about the type of its own state, and the compiler found all four the
// first time it was allowed to look at the template.
//
//   Input, Textarea  `string | number`
//       shadcn types the model as either, because the same component backs
//       `<input type="number">`.
//   Checkbox         `CheckboxCheckedState`
//       reka's tri-state, `boolean | "indeterminate"`. The label below renders
//       the third state as readily as the other two.
//   RadioGroup       `AcceptableValue`
//       reka's `string | number | bigint | Record | null`. The `null` — emitted
//       when the selection is cleared — is the one that mattered.
//   Tabs             `StringOrNumber`
//       `string | number`.
//
// Switch and Toggle are already exact: both emit plain `boolean`.
const text = ref<string | number>("editable — native gpui text input");
const notes = ref<string | number>("");
const checked = ref<CheckboxCheckedState>(true);
const on = ref(false);
const pressed = ref(false);
const plan = ref<AcceptableValue>("pro");
const tab = ref<StringOrNumber>("account");
</script>

<template>
  <div class="flex flex-col gap-4">
    <Section title="Input / Textarea" note="plain elements — no reka primitive">
      <Label for="name">Name</Label>
      <!--
        `text-foreground` is a WORKAROUND, not part of the upstream usage:
        vn-tree's native <input> falls back to WHITE when the element itself
        declares no `color` (crates/vn-tree/src/render.rs, build_input), and
        shadcn's Input deliberately inherits instead of declaring one — so on
        the white `bg-background` field the text is invisible. Passing the class
        through the component's own `class` prop is the least invasive fix; the
        real fix is inheriting the cascaded colour. See the report.
      -->
      <Input id="name" v-model="text" placeholder="Type here" class="text-foreground" />
      <Textarea v-model="notes" placeholder="Notes" />
      <div class="text-xs text-muted-foreground">model: {{ text }}</div>
    </Section>

    <Section title="Checkbox / Switch / Toggle" note="reka CheckboxRoot, SwitchRoot, Toggle">
      <div class="flex flex-row items-center gap-2">
        <Checkbox id="terms" v-model="checked" />
        <Label for="terms">Accept terms ({{ checked }})</Label>
      </div>
      <div class="flex flex-row items-center gap-2">
        <Switch id="mica" v-model="on" />
        <Label for="mica">Mica backdrop ({{ on }})</Label>
      </div>
      <Toggle v-model="pressed">Toggle ({{ pressed }})</Toggle>
    </Section>

    <Section title="RadioGroup" note="reka RadioGroupRoot + RovingFocus">
      <RadioGroup v-model="plan" class="flex flex-col gap-2">
        <div class="flex flex-row items-center gap-2">
          <RadioGroupItem id="r-free" value="free" />
          <Label for="r-free">Free</Label>
        </div>
        <div class="flex flex-row items-center gap-2">
          <RadioGroupItem id="r-pro" value="pro" />
          <Label for="r-pro">Pro</Label>
        </div>
      </RadioGroup>
      <div class="text-xs text-muted-foreground">selected: {{ plan }}</div>
    </Section>

    <Section title="Tabs" note="reka TabsRoot — data-[state=active] styling">
      <Tabs v-model="tab">
        <TabsList>
          <TabsTrigger value="account">Account</TabsTrigger>
          <TabsTrigger value="password">Password</TabsTrigger>
        </TabsList>
        <TabsContent value="account">
          <div class="text-sm">Account panel — state lives in reka's TabsRoot.</div>
        </TabsContent>
        <TabsContent value="password">
          <div class="text-sm">Password panel — swapped by Presence.</div>
        </TabsContent>
      </Tabs>
    </Section>

    <Section title="Accordion" note="reka AccordionRoot + CollapsibleContent">
      <Accordion type="single" collapsible default-value="a">
        <AccordionItem value="a">
          <AccordionTrigger>Does it run unmodified?</AccordionTrigger>
          <AccordionContent>The sources are byte-identical to upstream.</AccordionContent>
        </AccordionItem>
        <AccordionItem value="b">
          <AccordionTrigger>What is missing?</AccordionTrigger>
          <AccordionContent>Layout measurement, focus, and CSS animations.</AccordionContent>
        </AccordionItem>
      </Accordion>
    </Section>
  </div>
</template>
