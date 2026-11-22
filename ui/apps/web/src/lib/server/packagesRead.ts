// The packages and the clients' sign-ins, read on this server so a first
// load renders with them: through the control proxy, with the browser's
// Tailscale login and never its cookies. On a navigation they arrive like
// any page's read (`Arrival`). System's summary and its Packages page read
// the same, under the same keys.
import type { ServerLoadEvent } from '@sveltejs/kit';
import { Arrival } from '#lib/arrival.ts';
import { PackagesApi } from '#lib/packages.ts';
import { RunsClient } from '#lib/runs.ts';
import { readSnapshot } from '#lib/snapshots.ts';
import { RunsStartRead } from './runsStart.ts';

export class PackagesRead {
  static async load({ request, isDataRequest }: ServerLoadEvent) {
    const fetcher = RunsStartRead.fetcher(request);
    // The clients' sign-ins fail when runs are off here.
    const [listing, clients] = await Promise.all([
      Arrival.over(
        'packages',
        readSnapshot('packages', () => new PackagesApi(fetcher).listing()),
        isDataRequest,
      ),
      Arrival.over(
        'packages:clients',
        readSnapshot('packages:clients', () => new RunsClient(fetcher).clients()),
        isDataRequest,
      ),
    ]);
    return { listing, clients };
  }
}
