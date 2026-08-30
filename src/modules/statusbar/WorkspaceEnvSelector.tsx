import { useEffect } from "react";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { IS_WINDOWS } from "@/lib/platform";
import {
  LOCAL_WORKSPACE,
  useWorkspaceEnvStore,
  type WorkspaceEnv,
  workspaceEnvLabel,
} from "@/modules/workspace";
import { Refresh01Icon, ServerStack03Icon } from "@hugeicons/core-free-icons";
import { HugeiconsIcon } from "@hugeicons/react";

type Props = {
  onSelect: (env: WorkspaceEnv) => void;
};

export function WorkspaceEnvSelector({ onSelect }: Props) {
  const env = useWorkspaceEnvStore((s) => s.env);
  const distros = useWorkspaceEnvStore((s) => s.distros);
  const loading = useWorkspaceEnvStore((s) => s.loading);
  const error = useWorkspaceEnvStore((s) => s.error);
  const refreshDistros = useWorkspaceEnvStore((s) => s.refreshDistros);
  const sshHosts = useWorkspaceEnvStore((s) => s.sshHosts);
  const sshLoading = useWorkspaceEnvStore((s) => s.sshLoading);
  const sshError = useWorkspaceEnvStore((s) => s.sshError);
  const refreshSshHosts = useWorkspaceEnvStore((s) => s.refreshSshHosts);

  // Hosts come from a local file, so read them once up front: off Windows the
  // selector only exists when there is somewhere else to go.
  useEffect(() => {
    void refreshSshHosts();
  }, [refreshSshHosts]);

  if (!IS_WINDOWS && sshHosts.length === 0 && env.kind !== "ssh") return null;

  const handleOpenChange = (open: boolean) => {
    if (!open) return;
    if (IS_WINDOWS && distros.length === 0 && !loading) void refreshDistros();
    if (sshHosts.length === 0 && !sshLoading) void refreshSshHosts();
  };

  const refreshAll = () => {
    if (IS_WINDOWS) void refreshDistros();
    void refreshSshHosts();
  };

  const localLabel = IS_WINDOWS ? "Windows" : "Local";
  const label = workspaceEnvLabel(env, localLabel);

  return (
    <DropdownMenu onOpenChange={handleOpenChange}>
      <DropdownMenuTrigger asChild>
        <button
          type="button"
          className="flex h-6 shrink-0 items-center gap-1 rounded-sm px-1.5 text-[11px] text-muted-foreground outline-none hover:bg-accent hover:text-foreground focus:outline-none focus-visible:outline-none focus-visible:ring-0 data-[state=open]:bg-accent data-[state=open]:text-foreground"
          title="Workspace environment"
        >
          <HugeiconsIcon
            icon={ServerStack03Icon}
            size={13}
            strokeWidth={1.75}
          />
          <span className="max-w-28 truncate">{label}</span>
        </button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="start" className="min-w-48">
        <DropdownMenuItem onSelect={() => onSelect(LOCAL_WORKSPACE)}>
          {IS_WINDOWS ? "Windows Local" : "Local"}
        </DropdownMenuItem>
        {IS_WINDOWS ? (
          <>
            <DropdownMenuSeparator />
            {distros.length === 0 ? (
              <DropdownMenuItem disabled>
                {loading
                  ? "Loading WSL distros..."
                  : error
                    ? "WSL unavailable"
                    : "No WSL distros found"}
              </DropdownMenuItem>
            ) : (
              distros.map((distro) => (
                <DropdownMenuItem
                  key={distro.name}
                  onSelect={() =>
                    onSelect({ kind: "wsl", distro: distro.name })
                  }
                >
                  WSL: {distro.name}
                </DropdownMenuItem>
              ))
            )}
          </>
        ) : null}
        <DropdownMenuSeparator />
        <DropdownMenuLabel className="text-[10px] uppercase tracking-wide text-muted-foreground">
          SSH
        </DropdownMenuLabel>
        {sshHosts.length === 0 ? (
          <DropdownMenuItem disabled>
            {sshLoading
              ? "Loading SSH hosts..."
              : sshError
                ? "SSH config unavailable"
                : "No hosts in ~/.ssh/config"}
          </DropdownMenuItem>
        ) : (
          sshHosts.map((host) => (
            <DropdownMenuItem
              key={host.alias}
              title={
                host.hostName
                  ? `${host.user ? `${host.user}@` : ""}${host.hostName}${host.port ? `:${host.port}` : ""}`
                  : undefined
              }
              onSelect={() => onSelect({ kind: "ssh", host: host.alias, path: "" })}
            >
              ssh: {host.alias}
            </DropdownMenuItem>
          ))
        )}
        <DropdownMenuSeparator />
        <DropdownMenuItem onSelect={refreshAll}>
          <HugeiconsIcon icon={Refresh01Icon} size={13} strokeWidth={1.75} />
          Refresh
        </DropdownMenuItem>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}
