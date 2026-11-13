<script setup lang="ts">
import Section from "./Section.vue";
import { Button } from "../components/ui/button";
import { Badge } from "../components/ui/badge";
import { Card, CardContent, CardDescription, CardFooter, CardHeader, CardTitle } from "../components/ui/card";
import { Alert, AlertDescription, AlertTitle } from "../components/ui/alert";
import { Separator } from "../components/ui/separator";
import { Skeleton } from "../components/ui/skeleton";
import { Label } from "../components/ui/label";
import { Progress } from "../components/ui/progress";
</script>

<template>
  <div class="flex flex-col gap-4">
    <Section title="Button" note="6 variants x 4 sizes — cva + tailwind-merge">
      <div class="flex flex-row flex-wrap gap-2">
        <Button>Default</Button>
        <Button variant="secondary">Secondary</Button>
        <Button variant="destructive">Destructive</Button>
        <Button variant="outline">Outline</Button>
        <Button variant="ghost">Ghost</Button>
        <Button variant="link">Link</Button>
      </div>
      <div class="flex flex-row flex-wrap items-center gap-2">
        <Button size="sm">Small</Button>
        <Button size="default">Default</Button>
        <Button size="lg">Large</Button>
        <Button disabled>Disabled</Button>
      </div>
    </Section>

    <Section title="Badge" note="cva variants, no primitives">
      <div class="flex flex-row flex-wrap gap-2">
        <Badge>Default</Badge>
        <Badge variant="secondary">Secondary</Badge>
        <Badge variant="destructive">Destructive</Badge>
        <Badge variant="outline">Outline</Badge>
      </div>
    </Section>

    <Section title="Card" note="6 sub-components, slot composition">
      <Card>
        <CardHeader>
          <CardTitle>Native window, browser component</CardTitle>
          <CardDescription>Rendered by gpui through a Vue custom renderer.</CardDescription>
        </CardHeader>
        <CardContent>
          <div class="text-sm text-card-foreground">
            The sources under src/components/ui are byte-identical to shadcn-vue.
          </div>
        </CardContent>
        <CardFooter>
          <Button size="sm" variant="outline">Footer action</Button>
        </CardFooter>
      </Card>
    </Section>

    <Section title="Alert" note="default + destructive">
      <Alert>
        <AlertTitle>Heads up</AlertTitle>
        <AlertDescription>This alert is the upstream component, unedited.</AlertDescription>
      </Alert>
      <Alert variant="destructive">
        <AlertTitle>Destructive</AlertTitle>
        <AlertDescription>Colour comes from the pre-resolved theme tokens.</AlertDescription>
      </Alert>
    </Section>

    <Section title="Separator / Skeleton / Label / Progress" note="reka-ui Separator, Label, ProgressRoot">
      <div class="flex flex-col gap-2">
        <div class="text-sm text-card-foreground">Above the separator</div>
        <Separator />
        <div class="text-sm text-muted-foreground">Below the separator</div>
      </div>
      <div class="flex flex-row items-center gap-3">
        <Skeleton class="h-10 w-10 rounded-full" />
        <div class="flex flex-col gap-2">
          <Skeleton class="h-4 w-[220px]" />
          <Skeleton class="h-4 w-[160px]" />
        </div>
      </div>
      <Label for="demo">A reka-ui Label</Label>
      <Progress :model-value="62" />
    </Section>
  </div>
</template>
