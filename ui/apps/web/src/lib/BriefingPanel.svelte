<!--
  Redline's pane in Ren: the generic pane (`RedlinePanel`), with the brief
  shown as Markdown and a foot saying where it goes: the session running in
  Ren's own source checkout (`REN_SOURCE_DIR`) when there is one, so marks
  follow the work under way, else a new session on the client last used;
  its menu offers the rest. A new session gets the brief as its first
  prompt; a running one as its next, queued after its turn while one runs.
  Sending opens the session. The client last used, and where the pane was
  left, are remembered in this browser only.
-->
<script lang="ts">
  import { untrack } from 'svelte';
  import { goto } from '$app/navigation';
  import { page } from '$app/state';
  import { harnessLabel } from './activity.ts';
  import Markdown from 'markdown/Markdown.svelte';
  import Picker, { type PickerGroup } from './Picker.svelte';
  import SignInSheet from './SignInSheet.svelte';
  import Icon from './Icon.svelte';
  import { SignIns } from './signIn.ts';
  import type { Revision } from './api.ts';
  import { Redline, type PageContext } from 'redline';
  import type { RedlineDesk } from 'redline/desk';
  import RedlinePanel from 'redline/RedlinePanel.svelte';
  import { Briefing, type Destination } from './briefing.ts';
  import type { Recipients } from './briefing.svelte.ts';
  import {
    APPROVAL_POLICIES,
    Compose,
    DRIVABLE,
    EFFORTS,
    Setup,
    type ApprovalPolicy,
    RunCommandError,
    Runs,
    RunsClient,
    Sessions,
    type DeskSession,
    type Drivable,
    type SessionId,
  } from './runs.ts';

  let {
    desk,
    recipients,
    source,
    revision,
  }: {
    desk: RedlineDesk;
    /** Where a brief can go, read from the daemon. */
    recipients: Recipients;
    /** Ren's source checkout, where the agent works; null when unset. */
    source: string | null;
    /** What the daemon serving this page was built from, once read. */
    revision: Revision | null;
  } = $props();

  const client = new RunsClient();
  const KEEP = 'ren.redline';

  /** The client this browser last started a session on, read before the
   * first frame so the pane opens as it will stay. */
  const remembered = recall();
  /** The client picked in this pane; until then, the one last used. */
  let chosen = $state<Drivable | null>(null);
  /** Where the operator pointed the brief in this pane; until then, the
   * session running in the checkout, else a new one. */
  let picked = $state<'new' | SessionId | null>(null);
  let error = $state<string | null>(null);
  let sending = $state(false);

  /** The clients read; until then the pane keeps its shape, with every
   * client waiting. */
  const read = $derived(
    recipients.destinations.state === 'read' ? recipients.destinations.value : null,
  );
  const ready = $derived(read?.ready ?? []);
  const offered = $derived<DeskSession[]>(read?.sessions ?? []);
  /** The clients shown: those ready once read, every one while reading. */
  const shown = $derived(read ? read.ready : DRIVABLE);

  /** The client and approvals this browser last started a session with: a
   * convenience, so a refusal is ignored. */
  function recall(): { harness: Drivable | null; approvals: ApprovalPolicy | null } {
    try {
      const value: unknown = JSON.parse(localStorage.getItem(KEEP) ?? '{}');
      const kept = value && typeof value === 'object' ? (value as Record<string, unknown>) : {};
      return {
        harness: DRIVABLE.find((drivable) => drivable === kept.harness) ?? null,
        approvals: APPROVAL_POLICIES.find((policy) => policy === kept.approvals) ?? null,
      };
    } catch {
      return { harness: null, approvals: null };
    }
  }
  function keep() {
    try {
      localStorage.setItem(KEEP, JSON.stringify({ harness, approvals }));
    } catch {
      // Not kept; the pane offers the first ready client, on its default, next time.
    }
  }

  /** The client a send goes to: the one picked, else the one last sent to
   * while it is ready here, else the first ready. Settled before the first
   * frame from what this browser kept, so it does not change on reading. */
  const harness = $derived<Drivable>(
    chosen ??
      (read
        ? (read.ready.find((drivable) => drivable === remembered.harness) ??
          read.ready[0] ??
          remembered.harness ??
          DRIVABLE[0])
        : (remembered.harness ?? DRIVABLE[0])),
  );

  /** Where a send goes: the place picked, else the session running in the
   * checkout (the one waiting for you, else working, else the latest), so
   * marks follow the work under way, else a new session. */
  const destination = $derived.by((): Destination => {
    if (picked === 'new') return { kind: 'new' };
    const session =
      picked !== null ? offered.find((entry) => entry.session === picked) : offered[0];
    return session ? { kind: 'session', session } : { kind: 'new' };
  });
  /** The client the brief goes to: the new session's, or the running one's. */
  const target = $derived(
    read?.clients.find(
      (entry) =>
        entry.harness === (destination.kind === 'new' ? harness : destination.session.harness),
    ) ?? null,
  );
  /** That client is signed out on the daemon's host: the brief would only
   * fail, so it is not sent until it is signed in. */
  const signedOut = $derived(target !== null && SignIns.signedOut(target.sign_in));
  /** Another client to send it to instead, offered, never switched to. */
  const instead = $derived(
    read && destination.kind === 'new' && signedOut
      ? Briefing.instead(read.ready, read.clients, harness)
      : null,
  );
  /** The client whose sign-in sheet is open. */
  let signingIn = $state<Drivable | null>(null);
  /** A client's sign-in, as the destination menu says it. */
  const hintOf = (drivable: Drivable) => {
    const entry = read?.clients.find((offer) => offer.harness === drivable);
    return entry ? (SignIns.hint(entry.sign_in) ?? undefined) : undefined;
  };
  /** The approval policies that client runs under. */
  const policies = $derived(
    APPROVAL_POLICIES.filter((policy) => target?.approvals.includes(policy) ?? false),
  );
  /** The approvals picked in this pane: a policy, or the client's default. */
  let pickedApprovals = $state<{ approvals: ApprovalPolicy | null } | null>(null);
  /** What a send runs under: for a new session, the policy picked, else the
   * one last started with while the client offers it, else its default; for
   * a running session, the policy picked, else the one it runs under. */
  const approvals = $derived.by((): ApprovalPolicy | null => {
    const wanted =
      pickedApprovals?.approvals ??
      (pickedApprovals
        ? null
        : destination.kind === 'new'
          ? remembered.approvals
          : (APPROVAL_POLICIES.find(
              (policy) => policy === destination.session.settings.approvals,
            ) ?? null));
    return wanted && (!read || policies.includes(wanted)) ? wanted : null;
  });
  /** Why the approvals cannot be chosen here, if they cannot. */
  const fixed = $derived(
    !read
      ? 'Reading the clients…'
      : destination.kind === 'session' && !target?.midway.includes('approvals')
        ? `${harnessLabel(destination.session.harness)} takes approvals only when it starts`
        : null,
  );
  const DEFAULT = '';
  const policyGroups = $derived<PickerGroup[]>([
    {
      label: destination.kind === 'new' ? 'The new session runs under' : 'From its next turn',
      options: [
        {
          value: DEFAULT,
          label: 'Default',
          detail: `${harnessLabel(target?.harness ?? harness)}’s own choice`,
        },
        ...policies.map((policy) => ({
          value: policy,
          label: Setup.policy(policy),
          detail: Setup.explain(policy),
        })),
      ],
    },
  ]);

  /** A running session's state, in a word, with its client. */
  const stand = (session: DeskSession) => {
    const client = harnessLabel(session.harness);
    const entry = read?.clients.find((offer) => offer.harness === session.harness);
    if (entry && SignIns.signedOut(entry.sign_in)) return `Signed out · ${client}`;
    const mark = Sessions.mark(session, client);
    return mark.label === client ? client : `${mark.label} · ${client}`;
  };
  /** The destination menu: a new session on each client, then the
   * sessions running in the checkout. */
  const NEW = 'new:';
  const groups = $derived<PickerGroup[]>([
    {
      label: 'New session',
      options: shown.map((drivable) => ({
        value: `${NEW}${drivable}`,
        label: `New ${harnessLabel(drivable)} session`,
        // Its sign-in under its name: the narrow chip keeps the name.
        detail: hintOf(drivable) ?? null,
      })),
    },
    ...(offered.length && source
      ? [
          {
            label: `Running in ${Runs.folder(source)}`,
            options: offered.map((session) => ({
              value: session.session,
              label: session.asked ?? 'Untitled session',
              detail: stand(session),
            })),
          },
        ]
      : []),
  ]);
  const chosenPlace = $derived(
    destination.kind === 'new' ? `${NEW}${harness}` : destination.session.session,
  );
  function choose(value: string) {
    const drivable = DRIVABLE.find((entry) => `${NEW}${entry}` === value);
    if (drivable) {
      chosen = drivable;
      picked = 'new';
    } else {
      picked = value as SessionId;
    }
  }
  /** What the send button says: a brief for a working session waits for
   * its turn to end. */
  const action = $derived(
    sending
      ? 'Sending…'
      : destination.kind === 'session' && destination.session.state?.phase === 'working'
        ? 'Queue'
        : 'Send',
  );

  // Start reading, if the toggle's hover has not; read again at the next
  // opening, since sessions come and go.
  untrack(() => recipients.prepare());
  $effect(() => () => recipients.close());

  const context = $derived.by((): PageContext => {
    const chosen = document.documentElement.dataset.theme;
    const dark = chosen ? chosen === 'dark' : matchMedia('(prefers-color-scheme: dark)').matches;
    return {
      path: `${page.url.pathname}${page.url.search}`,
      title: document.title,
      width: window.innerWidth,
      height: window.innerHeight,
      theme: dark ? 'dark' : 'light',
    };
  });
  const noted = $derived(Redline.noted(desk.marks));
  const brief = $derived(
    Briefing.brief(context, { route: page.route.id, revision }, noted, Briefing.place(destination)),
  );

  /** A new session in `checkout`, with the brief as its first prompt. */
  async function start(checkout: string): Promise<SessionId> {
    // A workspace of its own: a worktree of the checkout, its own branch,
    // ports, and data folder.
    const made = await client.createSession(harness, checkout, { approvals }, 'worktree');
    await client.toSession(made, { kind: 'prompt', text: brief, delivery: 'start' });
    return made;
  }

  /** The brief into an open session, as it stands now (read again: it may
   * have moved on since the pane read it); null when it cannot take one. */
  async function into(session: SessionId): Promise<SessionId | null> {
    const [bodies, live] = await Promise.all([client.sessions(), client.live()]);
    const body = bodies.find((entry) => entry.session === session);
    const now = body ? Sessions.desk(body, live) : null;
    if (!now || now.shelf !== 'open') {
      error = 'That session is no longer open.';
      return null;
    }
    const handoff = Briefing.handoff(Compose.session(now.state, []), brief);
    switch (handoff.kind) {
      case 'send':
        // A changed policy first, so the brief's turn runs under it.
        // Its model and effort stay as they are; one this page does not
        // know is left unchanged by not sending the change at all.
        const effort = EFFORTS.find((level) => level === now.settings.effort) ?? null;
        if (
          approvals !== now.settings.approvals &&
          !fixed &&
          (now.settings.effort === null || effort !== null)
        ) {
          await client.toSession(session, {
            kind: 'settings',
            model: now.settings.model,
            effort,
            approvals,
          });
        }
        await client.toSession(session, {
          kind: 'prompt',
          text: handoff.text,
          delivery: handoff.delivery,
        });
        return session;
      case 'wait':
        error = handoff.reason;
        return null;
      default: {
        const never: never = handoff;
        return never;
      }
    }
  }

  async function send() {
    error = null;
    if (!noted.length) {
      error = 'Mark something and write a note first.';
      return;
    }
    if (!source || signedOut) return;
    sending = true;
    try {
      const target =
        destination.kind === 'new' ? await start(source) : await into(destination.session.session);
      if (target === null) return;
      if (destination.kind === 'new') keep();
      desk.clear();
      desk.toggle();
      await goto(`/sessions?session=${target}`);
    } catch (failure) {
      error = failure instanceof RunCommandError ? failure.message : 'The brief was not sent.';
    } finally {
      sending = false;
    }
  }
</script>

<RedlinePanel {desk} {brief} keep="ren.redline.pane">
  {#snippet preview(text: string)}
    <div class="markdown-brief"><Markdown {text} /></div>
  {/snippet}
  {#snippet footer()}
    {#if source}
      {#if error}<p class="error" role="alert">{error}</p>{/if}
      {#if recipients.destinations.state === 'failed'}
        <p class="error" role="alert">{recipients.destinations.reason}</p>
      {/if}
      {#if signedOut && target}
        <p class="signed-out" role="status">
          <Icon name="lock" size={12} />
          <span
            >{harnessLabel(target.harness)} is signed out on the daemon’s host. {destination.kind ===
            'session'
              ? 'The brief waits here until it is signed in.'
              : 'Sign it in to send the brief.'}</span
          >
          {#if instead}
            {@const other = instead}
            <button
              class="link"
              onclick={() => {
                chosen = other;
                picked = 'new';
              }}>Send to {harnessLabel(other)}</button
            >
          {/if}
        </p>
      {/if}
      <!-- One row: where the brief goes, what it may do there, and Send. -->
      <div class="send">
        <span class="where">
          <Picker
            label="Where the brief goes"
            icon="arrow"
            {groups}
            value={chosenPlace}
            placeholder="New session"
            search={false}
            disabled={read ? null : 'Reading where it can go…'}
            onchange={choose}
          />
        </span>
        <span class="policy">
          <Picker
            label={destination.kind === 'new' ? 'Approvals' : 'Approvals from its next turn'}
            icon="shield"
            groups={policyGroups}
            value={approvals ?? DEFAULT}
            placeholder="Default"
            search={false}
            disabled={fixed ?? (policies.length ? null : 'This client never asks')}
            onchange={(value) =>
              (pickedApprovals = {
                approvals: APPROVAL_POLICIES.find((policy) => policy === value) ?? null,
              })}
          />
        </span>
        {#if signedOut && target && Runs.drivable(target.harness) && target.sign_in_methods.length > 0}
          <!-- Signed out: the one action that leads on is signing in. -->
          <button
            class="button primary redline-send"
            onclick={() => (signingIn = Runs.drivable(target.harness))}
            ><Icon name="lock" size={11} />Sign in</button
          >
        {:else}
          <button
            class="button primary redline-send"
            title={signedOut
              ? `${harnessLabel(target?.harness ?? harness)} is signed out`
              : action === 'Queue'
                ? 'Sent when its current turn ends'
                : null}
            disabled={sending ||
              !read ||
              signedOut ||
              (destination.kind === 'new' && !ready.length)}
            onclick={() => void send()}>{action}</button
          >
        {/if}
      </div>
    {:else}
      <p class="hint unset">
        Redline sends to Ren's own source checkout, which this UI server doesn't know. Set
        <code>REN_SOURCE_DIR</code> in its environment (<code>~/.config/ren/ui.env</code>) to the
        checkout, a folder under the daemon's <code>[runs]</code> roots, and restart it.
      </p>
    {/if}
    {#if signingIn}
      {@const signing = signingIn}
      <SignInSheet
        harness={signing}
        client={harnessLabel(signing)}
        methods={read?.clients.find((entry) => entry.harness === signing)?.sign_in_methods ?? []}
        start={(method) => client.startSignIn(signing, method)}
        answer={(answer) => client.signInAnswer(signing, answer)}
        wait={() => client.waitSignIn(signing)}
        cancel={() => client.cancelSignIn(signing)}
        onclose={() => {
          signingIn = null;
          recipients.reread();
        }}
      />
    {/if}
  {/snippet}
</RedlinePanel>

<style>
  /* The brief as the agent reads it, at the pane's size; its line breaks
     kept, as the agent gets them. */
  .markdown-brief :global(.markdown) {
    font-size: 12px;
    line-height: 1.5;
  }
  .markdown-brief :global(.markdown p) {
    white-space: pre-wrap;
  }
  .markdown-brief :global(.markdown code) {
    padding: 0 3px;
    font-size: 11px;
  }
  .send {
    display: flex;
    align-items: center;
    gap: 8px;
    min-width: 0;
  }
  .where {
    flex: 1;
    min-width: 0;
  }
  .policy {
    flex: none;
    max-width: 45%;
  }
  .redline-send {
    flex: none;
  }
  /* The menus open from the pane's foot, not from their chips, so both
     span the pane's width wherever their chip sits in the row. */
  .send :global(.picker) {
    position: static;
    display: flex;
    min-width: 0;
  }
  .send :global(.picker .chip-button) {
    width: 100%;
    min-width: 0;
  }
  /* One line: the title cut short, its state kept whole. */
  .send :global(.chip-button > span:not(.hint)) {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    text-align: left;
  }
  .send :global(.chip-button .hint) {
    flex: none;
    white-space: nowrap;
  }
  /* The menu fits the pane instead of the page. */
  .send :global(.picker .panel) {
    left: 12px;
    width: 316px;
    max-width: calc(100vw - 24px);
  }
  .hint {
    margin: 0;
    color: var(--muted);
    font-size: 12px;
  }
  .unset code {
    font:
      11px 'SFMono-Regular',
      Consolas,
      monospace;
  }
  .error {
    margin: 0;
    color: var(--redline);
    font-size: 12px;
  }
  /* The destination's client is signed out: said above the send row. */
  .signed-out {
    display: flex;
    align-items: flex-start;
    gap: 6px;
    margin: 0;
    padding: 6px 8px;
    border: 1px solid var(--warning-border);
    background: var(--warning-soft);
    color: var(--warning);
    font-size: 12px;
    line-height: 1.4;
  }
  .signed-out :global(svg) {
    flex: none;
    margin-top: 2px;
  }
  .signed-out span {
    flex: 1;
  }
  .signed-out .link {
    flex: none;
    padding: 0;
    border: 0;
    background: none;
    color: var(--accent-text);
    font: inherit;
    text-decoration: underline;
    cursor: pointer;
  }
  .redline-send,
  .redline-send:hover:not(:disabled) {
    background: var(--redline);
    border-color: var(--redline);
    color: var(--on-redline);
  }
  .redline-send:hover:not(:disabled) {
    background: var(--redline-hover);
    border-color: var(--redline-hover);
  }
</style>
