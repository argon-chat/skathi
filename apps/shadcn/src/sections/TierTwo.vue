<script setup lang="ts">
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

// Input, Textarea `string | number` shadcn types the model as either, because the same component
// backs `<input type="number">`.
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
