<script lang="ts">
  import { untrack } from 'svelte';
  import Icon from '#lib/Icon.svelte';
  import { Questions, type Draft, type Question } from '#lib/questions.ts';
  import type { Decision } from '#lib/runs.ts';
  import type { ReplyRequest } from '#lib/wire/ReplyRequest.ts';

  // The questions a run's client waits on: each answered in its own form, then
  // sent together; or skipped, the turn going on or stopping.
  let {
    title,
    questions,
    index,
    busy,
    onreply,
    ondecide,
  }: {
    title: string | null;
    questions: Question[];
    index: number;
    busy: boolean;
    onreply: (request: ReplyRequest) => void;
    ondecide: (decision: Decision) => void;
  } = $props();

  // A fresh draft for each question, kept while the card stays: each
  // approval has its own card, so its questions are read once.
  let drafts: Draft[] = $state(
    untrack(() => questions.map((question) => Questions.draft(question))),
  );
  const answered = $derived(Questions.answered(questions, drafts));

  function set(at: number, draft: Draft) {
    drafts[at] = draft;
  }

  function submit(event: SubmitEvent) {
    event.preventDefault();
    if (!answered || busy) return;
    onreply(Questions.request(questions, drafts));
  }
</script>

<form class="questions" aria-label="Questions {index + 1}" onsubmit={submit}>
  <strong class="questions-title"
    ><Icon name="sessions" size={13} />
    {title ?? (questions.length === 1 ? 'A question for you' : 'Questions for you')}</strong
  >
  {#each questions as question, at (question.key)}
    {@const draft = drafts[at] ?? Questions.draft(question)}
    {@const form = question.form}
    <fieldset class="question">
      <legend>
        {#if question.header}<span class="question-header">{question.header}</span>{/if}
        <span class="question-text">{question.text}</span>
      </legend>
      {#if question.detail}<p class="muted question-detail">{question.detail}</p>{/if}
      {#if form.form === 'choice' && draft.form === 'choice'}
        {@const single = Questions.single(form)}
        <div class="options" role={single ? 'radiogroup' : 'group'}>
          {#each form.options as option (option.label)}
            {@const picked = draft.picked.includes(option.label)}
            <button
              type="button"
              class="option"
              class:picked
              role={single ? 'radio' : 'checkbox'}
              aria-checked={picked}
              disabled={busy}
              onclick={() => set(at, Questions.pick(question, draft, option.label))}
            >
              <span class="option-mark" aria-hidden="true"
                >{#if picked}<Icon name="check" size={11} />{/if}</span
              >
              <span class="option-text">
                <span class="option-label">{option.label}</span>
                {#if option.description}<span class="muted">{option.description}</span>{/if}
              </span>
            </button>
          {/each}
        </div>
        {#if Questions.writesIn(form)}
          <input
            class="write-in"
            type="text"
            placeholder={single ? 'Or answer in your own words' : 'Add your own answer'}
            aria-label="Your own answer to: {question.text}"
            value={draft.written}
            disabled={busy}
            oninput={(event) =>
              set(at, Questions.write(question, draft, event.currentTarget.value))}
          />
        {/if}
      {:else if form.form === 'text' && draft.form === 'text'}
        {#if form.lines === 'many' && !Questions.hidden(form)}
          <textarea
            rows="3"
            placeholder={form.placeholder ?? ''}
            aria-label={question.text}
            value={draft.text}
            disabled={busy}
            oninput={(event) => set(at, { form: 'text', text: event.currentTarget.value })}
          ></textarea>
        {:else}
          <input
            type={Questions.hidden(form) ? 'password' : 'text'}
            autocomplete="off"
            placeholder={form.placeholder ?? ''}
            aria-label={question.text}
            value={draft.text}
            disabled={busy}
            oninput={(event) => set(at, { form: 'text', text: event.currentTarget.value })}
          />
        {/if}
        {#if Questions.hidden(form)}<span class="muted">Sent to the client and kept nowhere.</span
          >{/if}
      {:else if form.form === 'number' && draft.form === 'number'}
        <label class="number">
          <input
            type="number"
            min={form.min}
            max={form.max}
            step={form.step ?? 'any'}
            aria-label={question.text}
            value={draft.value ?? ''}
            disabled={busy}
            oninput={(event) => {
              const value = event.currentTarget.valueAsNumber;
              set(at, { form: 'number', value: Number.isFinite(value) ? value : null });
            }}
          />
          <span class="muted">{form.unit ? `${form.unit}, ` : ''}{form.min} to {form.max}</span>
        </label>
      {:else if form.form === 'confirm' && draft.form === 'confirm'}
        <div class="options" role="radiogroup">
          {#each ['yes', 'no'] as const as answer (answer)}
            {@const picked = draft.answer === answer}
            <button
              type="button"
              class="option"
              class:picked
              role="radio"
              aria-checked={picked}
              disabled={busy}
              onclick={() => set(at, { form: 'confirm', answer: picked ? null : answer })}
            >
              <span class="option-mark" aria-hidden="true"
                >{#if picked}<Icon name="check" size={11} />{/if}</span
              >
              <span class="option-label">{answer === 'yes' ? 'Yes' : 'No'}</span>
            </button>
          {/each}
        </div>
      {:else}
        <span class="muted">This client asks in a way Ren does not know; it will be skipped.</span>
      {/if}
    </fieldset>
  {/each}
  <div class="questions-actions">
    {#if questions.length > 1}<span class="muted">Questions left blank are skipped.</span>{/if}
    <button class="button primary small" type="submit" disabled={busy || !answered}>Send</button>
    <button
      class="button small"
      type="button"
      disabled={busy}
      onclick={() => ondecide('decline')}
      title="Answer nothing; the turn goes on">Skip</button
    >
    <button
      class="button small"
      type="button"
      disabled={busy}
      onclick={() => ondecide('cancel')}
      title="Answer nothing and end the turn">Skip and stop</button
    >
  </div>
</form>

<style>
  .questions {
    display: flex;
    flex-direction: column;
    gap: 10px;
    padding: 10px 16px;
    font-size: 12px;
  }
  .questions-title {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    color: var(--warning);
  }
  .question {
    display: flex;
    flex-direction: column;
    gap: 6px;
    margin: 0;
    padding: 0;
    border: 0;
    min-width: 0;
  }
  legend {
    display: flex;
    align-items: baseline;
    gap: 8px;
    padding: 0;
    margin-bottom: 2px;
  }
  .question-header {
    flex: none;
    padding: 1px 6px;
    border: 1px solid var(--warning-border);
    background: var(--surface);
    font-size: 11px;
    color: var(--muted);
  }
  .question-text {
    font-weight: 600;
    color: var(--ink);
  }
  .question-detail {
    margin: 0;
  }
  /* Options sit flush, one rule between each. */
  .options {
    display: flex;
    flex-direction: column;
    border: 1px solid var(--warning-border);
    background: var(--surface);
  }
  .option {
    display: flex;
    align-items: flex-start;
    gap: 8px;
    padding: 6px 8px;
    border: 0;
    background: transparent;
    color: var(--ink);
    font: inherit;
    text-align: left;
    cursor: pointer;
  }
  .option + .option {
    border-top: 1px solid var(--line-subtle);
  }
  .option:hover:not(:disabled) {
    background: var(--warning-soft);
  }
  .option.picked {
    background: var(--warning-soft);
  }
  .option:disabled {
    cursor: default;
  }
  .option-mark {
    flex: none;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 14px;
    height: 14px;
    margin-top: 1px;
    border: 1px solid var(--line);
    color: var(--warning);
  }
  .option.picked .option-mark {
    border-color: var(--warning);
  }
  .option-text {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
  }
  .option-label {
    font-weight: 500;
  }
  .write-in,
  textarea {
    width: 100%;
  }
  textarea {
    resize: vertical;
    min-height: 60px;
    padding: 5px 8px;
    border: 1px solid var(--control-border);
    background: var(--surface);
    color: inherit;
    font: inherit;
  }
  .number {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .number input {
    width: 120px;
  }
  .questions-actions {
    display: flex;
    align-items: center;
    gap: 6px;
    flex-wrap: wrap;
    justify-content: flex-end;
  }
  .questions-actions > .muted {
    margin-right: auto;
  }
</style>
