/**
 * Shared record of every routed-view probe instance that was initialised,
 * with the props it received at init. WorkPane.test.ts reads it to prove a
 * navigation never mounts the PREVIOUS view's module with the new view's
 * props (drop-dup fix).
 */
export const routeProbeLog: Array<{ probe: string; props: Record<string, unknown> }> = []
