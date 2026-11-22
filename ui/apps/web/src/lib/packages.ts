// The packages Ren installs for its agents on the daemon's host: what the
// daemon's `/control/v1/packages` says (through this UI server's
// `/control/packages` proxy), and the Packages view's pure logic: each package's
// standing, the change the operator stages before applying it, and how the
// table groups packages. No browser APIs beyond the `fetch` handed in;
// tested in tests/packages.test.ts.
import { API_VERSION, harnesses, versioned, type Harness } from './api.ts';
import {
  ShapeError,
  array,
  assertNever,
  unrecognised,
  branded,
  brandedCount,
  isRecord,
  label,
  nullable,
  object,
  string,
  type Brand,
  type Stored,
  type AnyDecoder,
} from './decode.ts';
import type { Mark } from './marks.ts';
import { Refusals, type Refusal } from './refusal.ts';
import type { GenerationInstall as WireGenerationInstall } from './wire/GenerationInstall.ts';
import type { GenerationView as WireGenerationView } from './wire/GenerationView.ts';
import type { InstallFailure as WireInstallFailure } from './wire/InstallFailure.ts';
import type { InstalledPackage as WireInstalledPackage } from './wire/InstalledPackage.ts';
import type { PackageChange as WirePackageChange } from './wire/PackageChange.ts';
import type { SystemProgram as WireSystemProgram } from './wire/SystemProgram.ts';
import type { PackageSetChange as WirePackageSetChange } from './wire/PackageSetChange.ts';
import type { PackageCollection as WirePackageCollection } from './wire/PackageCollection.ts';
import type { PackageListing as WirePackageListing } from './wire/PackageListing.ts';
import type { ListedPackage as WireListedPackage } from './wire/ListedPackage.ts';
import type { ChangeRequest as WireChangeRequest } from './wire/ChangeRequest.ts';
import type { RollbackRequest as WireRollbackRequest } from './wire/RollbackRequest.ts';

/** A package's name in Ren's catalog (`node`, `jq`). */
export type PackageId = Brand<string, 'PackageId'>;
/** A release's version as its publisher numbers it. */
export type Version = Brand<string, 'Version'>;
/** A generation's number; 0 before anything is installed. */
export type Generation = Brand<number, 'Generation'>;

export const KINDS = ['runtime', 'utility', 'client'] as const;
export type Kind = (typeof KINDS)[number];
export const origins = ['builtin', 'operator'] as const;
/** Which catalog names a package: Ren's own, or the operator's `[tools]`. */
export type Origin = (typeof origins)[number];
export const platforms = ['linux_x64', 'linux_arm64', 'macos_arm64'] as const;
export type Platform = (typeof platforms)[number];
export const generationStates = ['current', 'in_use', 'idle'] as const;
export type GenerationState = (typeof generationStates)[number];
export const outcomes = ['unchanged', 'activated', 'failed'] as const;
export type ChangeOutcome = (typeof outcomes)[number];
export const changeKinds = ['add', 'replace', 'remove'] as const;
export type ChangeKind = (typeof changeKinds)[number];
export const stages = ['fetch', 'verify', 'unpack', 'place'] as const;
export type Stage = (typeof stages)[number];

export interface InstalledPackage {
  version: Version;
  sha256: string;
  dir: string;
}

export class InstalledPackages {
  /** An install's SHA-256 as the details pane shows it: its first twelve
   * digits, the whole on hover. */
  static shortSha(installed: InstalledPackage): string {
    return `${installed.sha256.slice(0, 12)}…`;
  }
}

export class Origins {
  /** Which catalog names a package, as its row says it: `removed` for an
   * install the catalog no longer names. */
  static label(origin: Stored<Origin> | null): string {
    if (origin === null) return 'removed';
    switch (origin) {
      case 'builtin':
      case 'operator':
        return origin;
      default:
        return unrecognised(origin);
    }
  }
}

export class GenerationStates {
  /** A generation's state as its column head says it: nothing for an idle one. */
  static label(state: Stored<GenerationState>): string {
    switch (state) {
      case 'current':
        return 'current';
      case 'in_use':
        return 'in use';
      case 'idle':
        return '';
      default:
        return unrecognised(state);
    }
  }
}

export class ChangeKinds {
  /** The sign a confirmed change carries: added, removed, or brought up. */
  static sign(kind: ChangeKind): string {
    switch (kind) {
      case 'add':
        return '+';
      case 'remove':
        return '−';
      case 'replace':
        return '↑';
      default:
        return assertNever(kind);
    }
  }
}

export interface SystemProgram {
  path: string;
  /** The version it reported, when the daemon could read one. */
  version: Version | null;
  /** What it printed instead, or why it did not answer. */
  unread: string | null;
}

export interface Package {
  id: PackageId;
  /** Which catalog names it; `null` for an install the catalog no longer names. */
  origin: Stored<Origin> | null;
  /** What it is for; `null` when the catalog no longer names it. */
  kind: Stored<Kind> | null;
  /** The program it provides (`rg` for `ripgrep`). */
  program: string;
  /** The release Ren installs; `null` for a package it only finds on PATH. */
  release: Version | null;
  installed: InstalledPackage | null;
  system: SystemProgram | null;
  needed_by: Stored<Harness>[];
}

export interface GenerationInstall {
  package: PackageId;
  version: Version;
}

export interface GenerationView {
  generation: Generation;
  state: Stored<GenerationState>;
  installs: GenerationInstall[];
}

export interface PackageListing {
  /** Where Ren installs packages; `null` on a platform it installs nothing for. */
  platform: Stored<Platform> | null;
  generation: Generation;
  /** The current generation's folder of programs; `null` before any. */
  bin: string | null;
  packages: Package[];
  generations: GenerationView[];
}

export interface PackageChange {
  change: Stored<ChangeKind>;
  package: PackageId;
  from: Version | null;
  to: Version | null;
}

export interface InstallFailure {
  package: PackageId;
  version: Version;
  stage: Stored<Stage>;
  reason: string;
}

export interface GenerationChange {
  outcome: Stored<ChangeOutcome>;
  generation: Generation;
  changes: PackageChange[];
  failures: InstallFailure[];
}

export class GenerationChanges {
  /** What a change came to, as a toast says it: activated, nothing to do, or
   * which installs failed and at which stage. */
  static said(change: GenerationChange): { mark: Mark; text: string; detail: string | null } {
    const outcome = change.outcome;
    switch (outcome) {
      case 'activated':
        return {
          mark: 'passed',
          text: `Generation ${change.generation} is current`,
          detail:
            change.changes.map((c) => `${c.package} ${c.to ?? 'removed'}`).join(' · ') || null,
        };
      case 'unchanged':
        return { mark: 'idle', text: 'Nothing to change', detail: null };
      case 'failed':
        return {
          mark: 'failed',
          text:
            change.failures.length === 1
              ? '1 install failed'
              : `${change.failures.length} installs failed`,
          detail: change.failures.map((f) => `${f.package} · ${f.stage}`).join(' · ') || null,
        };
      default:
        return { mark: 'unknown', text: unrecognised(outcome), detail: null };
    }
  }
}

export interface PackageCollection {
  generations: Generation[];
  installs: string[];
}

const packageId = branded<'PackageId'>();
const version = branded<'Version'>();
const generation = brandedCount<'Generation'>();

const listing = versioned(
  object<PackageListing, WirePackageListing>({
    platform: nullable(label(platforms)),
    generation,
    bin: nullable(string),
    packages: array(
      object<Package, WireListedPackage>({
        id: packageId,
        origin: nullable(label(origins)),
        kind: nullable(label(KINDS)),
        program: string,
        release: nullable(version),
        installed: nullable(
          object<InstalledPackage, WireInstalledPackage>({ version, sha256: string, dir: string }),
        ),
        system: nullable(
          object<SystemProgram, WireSystemProgram>({
            path: string,
            version: nullable(version),
            unread: nullable(string),
          }),
        ),
        needed_by: array(label(harnesses)),
      }),
    ),
    generations: array(
      object<GenerationView, WireGenerationView>({
        generation,
        state: label(generationStates),
        installs: array(
          object<GenerationInstall, WireGenerationInstall>({ package: packageId, version }),
        ),
      }),
    ),
  }),
);

const change = versioned(
  object<GenerationChange, WirePackageSetChange>({
    outcome: label(outcomes),
    generation,
    changes: array(
      object<PackageChange, WirePackageChange>({
        change: label(changeKinds),
        package: packageId,
        from: nullable(version),
        to: nullable(version),
      }),
    ),
    failures: array(
      object<InstallFailure, WireInstallFailure>({
        package: packageId,
        version,
        stage: label(stages),
        reason: string,
      }),
    ),
  }),
);

const collection = versioned(
  object<PackageCollection, WirePackageCollection>({
    generations: array(generation),
    installs: array(string),
  }),
);

/** Where a package stands on the daemon's host. */
export type Standing =
  /** Ren installed the catalog's release. */
  | { kind: 'installed'; version: Version }
  /** Ren installed another release than the catalog's. */
  | { kind: 'outdated'; installed: Version; release: Version }
  /** Ren can install it and has not. */
  | { kind: 'available'; release: Version }
  /** Ren only finds it on PATH, and it is there. */
  | { kind: 'system' }
  /** Ren only finds it on PATH, and it is not there. */
  | { kind: 'missing' }
  /** Installed, and since taken out of the catalog: it can only be removed. */
  | { kind: 'uncatalogued'; installed: Version };

export class Standings {
  static of(pkg: Package): Standing {
    if (pkg.origin === null && pkg.installed !== null) {
      return { kind: 'uncatalogued', installed: pkg.installed.version };
    }
    if (pkg.release === null) return pkg.system === null ? { kind: 'missing' } : { kind: 'system' };
    if (pkg.installed === null) return { kind: 'available', release: pkg.release };
    return pkg.installed.version === pkg.release
      ? { kind: 'installed', version: pkg.release }
      : { kind: 'outdated', installed: pkg.installed.version, release: pkg.release };
  }
}

/**
 * Where a new run's copy of a package comes from: Ren's install, first on
 * its PATH (over any copy the daemon's PATH has), the daemon's PATH, or
 * nowhere.
 */
export type Supply =
  | { kind: 'managed'; version: Version; shadows: SystemProgram | null }
  | { kind: 'system'; system: SystemProgram }
  | { kind: 'none' };

/** How many packages stand each way, for the view's header. */
export interface Tally {
  managed: number;
  system: number;
  /** Packages a client needs that runs would not find. */
  missing: number;
  /** Ren installs that are not the catalog's release. */
  updates: number;
}

export class Supplies {
  static of(pkg: Package): Supply {
    if (pkg.installed !== null) {
      return { kind: 'managed', version: pkg.installed.version, shadows: pkg.system };
    }
    if (pkg.system !== null) return { kind: 'system', system: pkg.system };
    return { kind: 'none' };
  }

  /** The row's dot; `missing` is nothing where a client needs it. */
  static tone(pkg: Package): 'managed' | 'system' | 'none' | 'missing' {
    const supply = Supplies.of(pkg);
    switch (supply.kind) {
      case 'managed':
      case 'system':
        return supply.kind;
      case 'none':
        return pkg.needed_by.length > 0 ? 'missing' : 'none';
      default:
        return assertNever(supply);
    }
  }

  /** How a package stands, as a mark and its words: Ren's install of the
   * catalog's release passes, another release needs a look, a package a
   * client needs and runs would not find failed, and any other rests. */
  static mark(pkg: Package): { mark: Mark; word: string } {
    if (Standings.of(pkg).kind === 'outdated') return { mark: 'warning', word: 'Update available' };
    const tone = Supplies.tone(pkg);
    switch (tone) {
      case 'managed':
        return { mark: 'passed', word: 'Installed by Ren' };
      case 'system':
        return { mark: 'idle', word: 'On the host’s PATH' };
      case 'none':
        return { mark: 'idle', word: 'Not installed' };
      case 'missing':
        return { mark: 'failed', word: 'Missing' };
      default:
        return assertNever(tone);
    }
  }

  /** The version runs use: `?` when the PATH's copy did not say. */
  static version(supply: Supply): Version | '?' | null {
    switch (supply.kind) {
      case 'managed':
        return supply.version;
      case 'system':
        return supply.system.version ?? '?';
      case 'none':
        return null;
      default:
        return assertNever(supply);
    }
  }

  /**
   * What runs use of `pkg` before and after `change`: an install or update
   * gives them Ren's release; a removal leaves them the PATH's copy, if any.
   */
  static effect(pkg: Package, change: PlannedChange): Effect {
    const after: Supply =
      change.change === 'remove'
        ? pkg.system === null
          ? { kind: 'none' }
          : { kind: 'system', system: pkg.system }
        : { kind: 'managed', version: change.to ?? ('' as Version), shadows: pkg.system };
    return {
      change: change.change,
      id: pkg.id,
      from: Supplies.version(Supplies.of(pkg)),
      to: Supplies.version(after),
      toKind: after.kind,
    };
  }

  static tally(listing: PackageListing): Tally {
    const tally: Tally = { managed: 0, system: 0, missing: 0, updates: 0 };
    for (const pkg of listing.packages) {
      const tone = Supplies.tone(pkg);
      if (tone !== 'none') tally[tone] += 1;
      if (Standings.of(pkg).kind === 'outdated') tally.updates += 1;
    }
    return tally;
  }
}

export class Platforms {
  /** A platform as people name it. */
  static label(platform: Stored<Platform>): string {
    switch (platform) {
      case 'linux_x64':
        return 'Linux x64';
      case 'linux_arm64':
        return 'Linux arm64';
      case 'macos_arm64':
        return 'macOS arm64';
      default:
        return platform;
    }
  }
}

/** The one change a row's button stages, with the version it brings. */
export type RowAction =
  { kind: 'install'; version: Version } | { kind: 'update'; version: Version } | { kind: 'remove' };

export class RowActions {
  /** What `pkg`'s button does; `null` for a package Ren does not install. */
  static of(pkg: Package): RowAction | null {
    const standing = Standings.of(pkg);
    switch (standing.kind) {
      case 'available':
        return { kind: 'install', version: standing.release };
      case 'outdated':
        return { kind: 'update', version: standing.release };
      case 'installed':
      case 'uncatalogued':
        return { kind: 'remove' };
      case 'system':
      case 'missing':
        return null;
      default:
        return assertNever(standing);
    }
  }

  /** The words on `pkg`'s button under `plan`: `Undo` once staged, the change
   * it stages otherwise; `null` for a package Ren does not install. */
  static label(pkg: Package, plan: PackagePlan): string | null {
    const row = RowActions.of(pkg);
    if (row === null) return null;
    if (plan.staged(pkg)) return 'Undo';
    switch (row.kind) {
      case 'install':
        return `Install ${row.version}`;
      case 'update':
        return `Update ${row.version}`;
      case 'remove':
        return 'Remove';
      default:
        return assertNever(row);
    }
  }
}

/** Whether a change is on its way; how it ended is a toast. */
export type Applying = { kind: 'idle' } | { kind: 'applying' };
/** The change the operator is asked to confirm, if any. */
export type Confirming = { kind: 'apply' } | { kind: 'restore'; generation: Generation } | null;

/** A staged change as runs see it: the version they use before and after. */
export interface Effect {
  change: ChangeKind;
  id: PackageId;
  from: Version | '?' | null;
  to: Version | '?' | null;
  /** Where runs get it after the change. */
  toKind: Supply['kind'];
}

/** One row of a staged plan, as it will change the installed packages. */
export interface PlannedChange {
  change: ChangeKind;
  id: PackageId;
  from: Version | null;
  to: Version | null;
}

/**
 * The change the operator stages before applying it: packages to add (or
 * bring to the catalog's release) and packages to remove. Immutable; each
 * toggle makes a new plan.
 */
export class PackagePlan {
  readonly add: ReadonlySet<PackageId>;
  readonly remove: ReadonlySet<PackageId>;

  constructor(add: Iterable<PackageId> = [], remove: Iterable<PackageId> = []) {
    this.add = new Set(add);
    this.remove = new Set(remove);
  }

  static readonly EMPTY = new PackagePlan();

  get empty(): boolean {
    return this.add.size === 0 && this.remove.size === 0;
  }

  /** Whether `pkg` is staged, either way. */
  staged(pkg: Package): boolean {
    return this.add.has(pkg.id) || this.remove.has(pkg.id);
  }

  /**
   * Stage or unstage the one change `pkg` can take: install it, update it to
   * the catalog's release, or remove an installed one. A package Ren only
   * finds on PATH takes none, and the plan is returned unchanged.
   */
  toggle(pkg: Package): PackagePlan {
    const add = new Set(this.add);
    const remove = new Set(this.remove);
    const flip = (set: Set<PackageId>) => (set.has(pkg.id) ? set.delete(pkg.id) : set.add(pkg.id));
    const standing = Standings.of(pkg);
    switch (standing.kind) {
      case 'available':
      case 'outdated':
        flip(add);
        break;
      case 'installed':
      case 'uncatalogued':
        flip(remove);
        break;
      case 'system':
      case 'missing':
        return this;
      default:
        return assertNever(standing);
    }
    return new PackagePlan(add, remove);
  }

  /** Stage removing an outdated install instead of updating it. */
  toggleRemove(pkg: Package): PackagePlan {
    if (pkg.installed === null) return this;
    const add = new Set(this.add);
    const remove = new Set(this.remove);
    add.delete(pkg.id);
    if (remove.has(pkg.id)) remove.delete(pkg.id);
    else remove.add(pkg.id);
    return new PackagePlan(add, remove);
  }

  /** What applying this plan to `listing` changes, in listing order. */
  changes(listing: PackageListing): PlannedChange[] {
    const planned: PlannedChange[] = [];
    for (const pkg of listing.packages) {
      if (this.remove.has(pkg.id) && pkg.installed !== null) {
        planned.push({ change: 'remove', id: pkg.id, from: pkg.installed.version, to: null });
      } else if (this.add.has(pkg.id) && pkg.release !== null) {
        if (pkg.installed === null) {
          planned.push({ change: 'add', id: pkg.id, from: null, to: pkg.release });
        } else if (pkg.installed.version !== pkg.release) {
          planned.push({
            change: 'replace',
            id: pkg.id,
            from: pkg.installed.version,
            to: pkg.release,
          });
        }
      }
    }
    return planned;
  }

  /** What applying this plan changes for runs, in listing order. */
  effects(listing: PackageListing): Effect[] {
    const byId = new Map(listing.packages.map((p) => [p.id, p]));
    return this.changes(listing).flatMap((change) => {
      const pkg = byId.get(change.id);
      return pkg ? [Supplies.effect(pkg, change)] : [];
    });
  }

  /** Only what still applies to `listing`, after it changed. */
  within(listing: PackageListing): PackagePlan {
    const changes = this.changes(listing);
    return new PackagePlan(
      changes.filter((c) => c.change !== 'remove').map((c) => c.id),
      changes.filter((c) => c.change === 'remove').map((c) => c.id),
    );
  }

  /** The edit `/control/packages/change` takes. */
  body(): { add: PackageId[]; remove: PackageId[] } & WireChangeRequest {
    return { add: [...this.add].sort(), remove: [...this.remove].sort() };
  }
}

/**
 * Packages grouped by kind, in the order the table shows them; installs the
 * catalog no longer names (`kind` null) come last.
 */
export interface KindGroup {
  kind: Stored<Kind> | null;
  title: string;
  packages: Package[];
}

/** A header chip's narrowing: one way packages stand. */
export type Only = 'managed' | 'system' | 'missing' | 'updates';

/** Each narrowing's mark, the same its packages carry. */
export const ONLY_MARKS: Record<Only, Mark> = {
  managed: 'passed',
  system: 'idle',
  missing: 'failed',
  updates: 'warning',
};

/** What the view shows: packages matching `query`, narrowed to `only`. */
export interface PackageFilter {
  query: string;
  only: Only | null;
}

export class PackageFilters {
  static readonly ALL: PackageFilter = { query: '', only: null };

  /** Whether `pkg` matches: its name, program, or path holds the query. */
  static keeps(pkg: Package, filter: PackageFilter): boolean {
    const query = filter.query.trim().toLowerCase();
    if (
      query !== '' &&
      ![pkg.id, pkg.program, pkg.system?.path ?? ''].some((text) =>
        text.toLowerCase().includes(query),
      )
    ) {
      return false;
    }
    switch (filter.only) {
      case null:
        return true;
      case 'managed':
      case 'system':
      case 'missing':
        return Supplies.tone(pkg) === filter.only;
      case 'updates':
        return Standings.of(pkg).kind === 'outdated';
      default:
        return assertNever(filter.only);
    }
  }
}

export class PackageGroups {
  static of(listing: PackageListing, filter: PackageFilter = PackageFilters.ALL): KindGroup[] {
    const known: Stored<Kind>[] = [...KINDS];
    const unknown = new Set<Stored<Kind>>();
    for (const p of listing.packages) {
      if (p.kind !== null && !known.includes(p.kind)) unknown.add(p.kind);
    }
    const kinds: (Stored<Kind> | null)[] = [...known, ...unknown, null];
    return kinds
      .map((kind) => ({
        kind,
        title: PackageGroups.title(kind),
        packages: listing.packages.filter(
          (p) => p.kind === kind && PackageFilters.keeps(p, filter),
        ),
      }))
      .filter((group) => group.packages.length > 0);
  }

  static title(kind: Stored<Kind> | null): string {
    if (kind === null) return 'Not in the catalog';
    switch (kind) {
      case 'runtime':
        return 'Runtimes';
      case 'utility':
        return 'Utilities';
      case 'client':
        return 'Clients';
      default:
        return kind;
    }
  }
}

/** Why a packages request did not succeed, in the operator's words. */
export class PackagesError extends Error {
  readonly status: number;
  readonly code: Stored<Refusal> | null;

  constructor(status: number, code: Stored<Refusal> | null, message: string | null) {
    super(PackagesError.describe(status, code, message));
    this.status = status;
    this.code = code;
  }

  /** What a failed packages request says to the operator, whatever it threw. */
  static say(error: unknown): string {
    return error instanceof Error ? error.message : String(error);
  }

  /** The daemon's message where it explains more; otherwise the code's
   * words, said for packages where the proxy's refusals read otherwise. */
  static describe(status: number, code: Stored<Refusal> | null, message: string | null): string {
    if (code === null) return `The request was refused (${status}).`;
    switch (code) {
      case 'packages_refused':
      case 'packages_no_generation':
      case 'packages_collected':
        return message === null ? Refusals.say(code) : `${message}.`;
      case 'runs_off':
        return 'Set REN_CONTROL_TOKEN_FILE on the UI server to manage packages here.';
      case 'login_not_allowed':
        return 'Your Tailscale login may not change packages here.';
      default:
        return Refusals.say(code);
    }
  }
}

/** The daemon's packages, through this UI server's proxy. */
export class PackagesApi {
  readonly #fetch: typeof fetch;

  constructor(fetcher: typeof fetch) {
    this.#fetch = fetcher;
  }

  listing(signal?: AbortSignal): Promise<PackageListing> {
    return this.#request('GET', '/control/packages', undefined, listing, signal);
  }

  change(plan: PackagePlan): Promise<GenerationChange> {
    return this.#request('POST', '/control/packages/change', plan.body(), change);
  }

  /** Make the next generation hold what a kept `generation` held. */
  rollBack(generation: Generation): Promise<GenerationChange> {
    return this.#request(
      'POST',
      '/control/packages/rollback',
      { generation } satisfies WireRollbackRequest,
      change,
    );
  }

  collect(): Promise<PackageCollection> {
    return this.#request('POST', '/control/packages/gc', {}, collection);
  }

  async #request<T>(
    method: 'GET' | 'POST',
    path: string,
    body: unknown,
    decode: AnyDecoder<T>,
    signal?: AbortSignal,
  ): Promise<T> {
    const response = await this.#fetch(path, {
      method,
      cache: 'no-store',
      headers:
        body === undefined
          ? { accept: 'application/json' }
          : { accept: 'application/json', 'content-type': 'application/json' },
      body: body === undefined ? undefined : JSON.stringify(body),
      signal,
    });
    let parsed: unknown = null;
    try {
      parsed = await response.json();
    } catch {
      parsed = null;
    }
    if (!response.ok) {
      const code = Refusals.read(parsed);
      const message =
        isRecord(parsed) && typeof parsed.message === 'string' ? parsed.message : null;
      throw new PackagesError(response.status, code, message);
    }
    if (!isRecord(parsed) || parsed.api_version !== API_VERSION) {
      throw new ShapeError('api_version', `${API_VERSION}`);
    }
    return decode(parsed, '');
  }
}

/** The newest generations side by side, as columns beside the packages:
 * which release of each package each held, and where that changed from
 * the generation before it. */
export class PackageHistory {
  /** Newest first, at most `max`. */
  readonly generations: GenerationView[];
  readonly #versions: Map<Generation, Map<PackageId, Version>>;
  /** The generation just older than the oldest column, to tell a change in it. */
  readonly #before: Map<PackageId, Version> | null;

  constructor(listing: PackageListing, max = 5) {
    const newest = listing.generations.toSorted((a, b) => b.generation - a.generation);
    this.generations = newest.slice(0, max);
    this.#versions = new Map(
      newest.map((generation) => [
        generation.generation,
        new Map(generation.installs.map((install) => [install.package, install.version])),
      ]),
    );
    const before = newest[max];
    this.#before = before ? (this.#versions.get(before.generation) ?? null) : null;
  }

  /** The newest generation older than the current one: what Restore brings back. */
  static previous(listing: PackageListing): GenerationView | null {
    return (
      listing.generations
        .filter((generation) => generation.generation < listing.generation)
        .toSorted((a, b) => b.generation - a.generation)[0] ?? null
    );
  }

  /** The release of `pkg` that generation `index` (0 the newest) held. */
  version(pkg: PackageId, index: number): Version | null {
    const generation = this.generations[index];
    return generation ? (this.#versions.get(generation.generation)?.get(pkg) ?? null) : null;
  }

  /** Whether generation `index` changed `pkg` from the one before it. */
  changed(pkg: PackageId, index: number): boolean {
    const older =
      index + 1 < this.generations.length
        ? this.version(pkg, index + 1)
        : this.#before === null
          ? undefined
          : (this.#before.get(pkg) ?? null);
    return older !== undefined && older !== this.version(pkg, index);
  }
}
