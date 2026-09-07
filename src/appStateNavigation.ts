export type AppStateSaveHandler = () => Promise<unknown>;

export interface AppSuspendNavigationCoordinator {
  run: (saveState: AppStateSaveHandler | null, navigate: () => void) => void;
}

export function createAppSuspendNavigationCoordinator(): AppSuspendNavigationCoordinator {
  let suspension: Promise<void> | null = null;
  let pendingNavigation: (() => void) | null = null;

  return {
    run(saveState, navigate) {
      if (!saveState) {
        navigate();
        return;
      }

      pendingNavigation = navigate;
      if (suspension) return;

      const current = Promise.resolve()
        .then(saveState)
        .then(() => undefined)
        .catch(() => undefined)
        .finally(() => {
          if (suspension !== current) return;
          suspension = null;
          const next = pendingNavigation;
          pendingNavigation = null;
          next?.();
        });
      suspension = current;
    },
  };
}
