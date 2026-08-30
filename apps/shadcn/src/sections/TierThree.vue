<script setup lang="ts">
// TIER 3 — portal + floating components. These are the ones that reach deepest
// into the DOM: Teleport targets, focus traps, dismissable layers, and
// @floating-ui positioning driven by getBoundingClientRect.
//
// As of compat round 6 this tier needs NO app-level configuration at all. It
// used to wrap everything in reka-ui's `<ConfigProvider :teleport-to>`, because
// reka's `<Teleport>` defaults to the STRING "body" and the renderer had no
// `nodeOps.querySelector` to resolve it with. It has one now, and "body" means
// the app root — which is also what makes the dialog's `fixed inset-0` cover
// the window rather than one column of the page.
import { ref } from "vue";
import type { AcceptableValue } from "reka-ui";
import Section from "./Section.vue";
import { Button } from "../components/ui/button";
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle, DialogTrigger } from "../components/ui/dialog";
import { Tooltip, TooltipContent, TooltipProvider, TooltipTrigger } from "../components/ui/tooltip";
import { Popover, PopoverContent, PopoverTrigger } from "../components/ui/popover";
import { Select, SelectContent, SelectGroup, SelectItem, SelectLabel, SelectTrigger, SelectValue } from "../components/ui/select";
import { DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuLabel, DropdownMenuSeparator, DropdownMenuTrigger } from "../components/ui/dropdown-menu";

// `AcceptableValue`, not `string`: reka's SelectRoot writes `null` back through
// `v-model` when the selection is cleared, so a `Ref<string>` here was a lie the
// template type-checker caught the first time it was pointed at this file. See
// the same note in TierTwo.vue.
const fruit = ref<AcceptableValue>("apple");
</script>

<template>
  <div class="flex flex-col gap-4">
    <Section title="Dialog" note="Teleport + FocusScope + DismissableLayer">
      <Dialog>
        <DialogTrigger as-child>
          <Button variant="outline">Open dialog</Button>
        </DialogTrigger>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>Portalled content</DialogTitle>
            <DialogDescription>Rendered through Vue's Teleport into the app root.</DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <Button size="sm">Confirm</Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </Section>

    <Section title="Tooltip / Popover" note="@floating-ui positioning">
      <div class="flex flex-row gap-2">
        <TooltipProvider>
          <Tooltip>
            <TooltipTrigger as-child>
              <Button variant="outline" size="sm">Hover me</Button>
            </TooltipTrigger>
            <TooltipContent>Tooltip body</TooltipContent>
          </Tooltip>
        </TooltipProvider>

        <Popover>
          <PopoverTrigger as-child>
            <Button variant="outline" size="sm">Popover</Button>
          </PopoverTrigger>
          <PopoverContent>
            <div class="text-sm">Popover body</div>
          </PopoverContent>
        </Popover>
      </div>
    </Section>

    <Section title="Select" note="Collection + Teleport + typeahead">
      <Select v-model="fruit">
        <SelectTrigger>
          <SelectValue placeholder="Pick a fruit" />
        </SelectTrigger>
        <SelectContent>
          <SelectGroup>
            <SelectLabel>Fruits</SelectLabel>
            <SelectItem value="apple">Apple</SelectItem>
            <SelectItem value="banana">Banana</SelectItem>
          </SelectGroup>
        </SelectContent>
      </Select>
      <div class="text-xs text-muted-foreground">selected: {{ fruit }}</div>
    </Section>

    <Section title="DropdownMenu" note="Menu + RovingFocus + focus guards">
      <DropdownMenu>
        <DropdownMenuTrigger as-child>
          <Button variant="outline" size="sm">Open menu</Button>
        </DropdownMenuTrigger>
        <DropdownMenuContent>
          <DropdownMenuLabel>Account</DropdownMenuLabel>
          <DropdownMenuSeparator />
          <DropdownMenuItem>Profile</DropdownMenuItem>
          <DropdownMenuItem>Settings</DropdownMenuItem>
        </DropdownMenuContent>
      </DropdownMenu>
    </Section>
  </div>
</template>
