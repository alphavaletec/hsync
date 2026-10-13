<script lang="ts">
  // Logo do provedor da conta; ícone genérico quando a marca não tem logo disponível.
  import { Cloud, HardDrive, Server } from "@lucide/svelte";
  import { providerLogo, readableColor } from "../../utils/providerLogos";

  let { kind, size = 16, class: cls = "" }: { kind: string; size?: number; class?: string } = $props();

  const SERVER_KINDS = new Set(["sftp", "ftp", "smb", "webdav", "http", "s3", "swift", "hdfs"]);
  const logo = $derived(providerLogo(kind));
</script>

{#if logo}
  <svg
    viewBox="0 0 24 24"
    width={size}
    height={size}
    fill={readableColor(logo.hex)}
    class="shrink-0 {cls}"
    role="img"
    aria-label={logo.title}
  >
    <path d={logo.path} />
  </svg>
{:else if kind === "local"}
  <HardDrive {size} class="shrink-0 text-muted {cls}" />
{:else if SERVER_KINDS.has(kind)}
  <Server {size} class="shrink-0 text-muted {cls}" />
{:else}
  <Cloud {size} class="shrink-0 text-accent {cls}" />
{/if}
