// Questions a run's client asks the operator mid-turn (Claude Code's
// AskUserQuestion, Codex's request_user_input, a Pi extension's dialogs), as
// the daemon sends them, and the replies the operator drafts to them. Pure: no
// browser, no fetch.

import {
  array,
  label,
  nullable,
  object,
  ShapeError,
  string,
  tagged,
  tagValue,
  unrecognised,
  assertNever,
  isRecord,
  type Brand,
  type Decoder,
  type Stored,
  type Variant,
} from './decode.ts';
import type { Question as WireQuestion } from './wire/Question.ts';
import type { QuestionForm as WireQuestionForm } from './wire/QuestionForm.ts';
import type { QuestionOption as WireQuestionOption } from './wire/QuestionOption.ts';
import type { ReplyBody as WireReplyBody } from './wire/ReplyBody.ts';
import type { ReplyRequest as WireReplyRequest } from './wire/ReplyRequest.ts';

/** What a client names a question by, so a reply finds it. */
export type QuestionKey = Brand<string, 'QuestionKey'>;

export const picks = ['one', 'many'] as const;
export const writeIns = ['allowed', 'refused'] as const;
export const textLines = ['one', 'many'] as const;
export const echoes = ['shown', 'hidden'] as const;

export interface QuestionOption {
  label: string;
  description: string | null;
}

/** How a question is answered; a form a newer daemon sends can only be skipped. */
export type QuestionForm =
  | {
      form: 'choice';
      options: QuestionOption[];
      pick: Stored<(typeof picks)[number]>;
      write_in: Stored<(typeof writeIns)[number]>;
    }
  | {
      form: 'text';
      lines: Stored<(typeof textLines)[number]>;
      echo: Stored<(typeof echoes)[number]>;
      placeholder: string | null;
      prefill: string | null;
    }
  | {
      form: 'number';
      min: number;
      max: number;
      step: number | null;
      start: number | null;
      unit: string | null;
    }
  | { form: 'confirm' }
  | { form: 'unrecognized'; name: string };

export interface Question {
  key: QuestionKey;
  header: string | null;
  text: string;
  detail: string | null;
  form: QuestionForm;
}

/** The operator's draft answer to one question, as its form edits it. */
export type Draft =
  | { form: 'choice'; picked: string[]; written: string }
  | { form: 'text'; text: string }
  | { form: 'number'; value: number | null }
  | { form: 'confirm'; answer: 'yes' | 'no' | null }
  | { form: 'none' };

const number: Decoder<number, number> = (value, path) => {
  if (typeof value !== 'number' || !Number.isFinite(value)) throw new ShapeError(path, 'a number');
  return value;
};

const optionalString: Decoder<string | null, string | null> = (value, path) =>
  value === undefined || value === null ? null : string(value, path);

const questionOption = object<QuestionOption, WireQuestionOption>({
  label: string,
  description: optionalString,
});

type FormOf<K> = Extract<QuestionForm, { form: K }>;
type SentForm<K> = Variant<WireQuestionForm, 'form', K>;
const questionForm = tagged<QuestionForm, WireQuestionForm, 'form'>(
  'form',
  {
    choice: object<FormOf<'choice'>, SentForm<'choice'>>({
      form: tagValue('choice'),
      options: array(questionOption),
      pick: label(picks),
      write_in: label(writeIns),
    }),
    text: object<FormOf<'text'>, SentForm<'text'>>({
      form: tagValue('text'),
      lines: label(textLines),
      echo: label(echoes),
      placeholder: optionalString,
      prefill: optionalString,
    }),
    number: object<FormOf<'number'>, SentForm<'number'>>({
      form: tagValue('number'),
      min: number,
      max: number,
      step: nullable(number),
      start: nullable(number),
      unit: optionalString,
    }),
    confirm: object<FormOf<'confirm'>, SentForm<'confirm'>>({ form: tagValue('confirm') }),
  },
  (value, path) => {
    if (!isRecord(value)) throw new ShapeError(path, 'an object');
    return { form: 'unrecognized', name: String(value.form) };
  },
);

export class Questions {
  static readonly decode = object<Question, WireQuestion>({
    key: (value, path) => string(value, path) as QuestionKey,
    header: optionalString,
    text: string,
    detail: optionalString,
    form: questionForm,
  });

  /** The draft a question starts from: what its client prefilled, or nothing. */
  static draft(question: Question): Draft {
    const form = question.form;
    switch (form.form) {
      case 'choice':
        return { form: 'choice', picked: [], written: '' };
      case 'text':
        return { form: 'text', text: form.prefill ?? '' };
      case 'number':
        return { form: 'number', value: form.start };
      case 'confirm':
        return { form: 'confirm', answer: null };
      case 'unrecognized':
        return { form: 'none' };
      default:
        return assertNever(form);
    }
  }

  /** Whether one option is picked at most: a single choice, or one of a kind not known here. */
  static single(form: Extract<QuestionForm, { form: 'choice' }>): boolean {
    switch (form.pick) {
      case 'one':
        return true;
      case 'many':
        return false;
      default:
        unrecognised(form.pick);
        return true;
    }
  }

  /** Whether a choice takes an answer in the operator's own words. */
  static writesIn(form: Extract<QuestionForm, { form: 'choice' }>): boolean {
    switch (form.write_in) {
      case 'allowed':
        return true;
      case 'refused':
        return false;
      default:
        unrecognised(form.write_in);
        return false;
    }
  }

  /** Whether a text answer is a secret, typed unseen. */
  static hidden(form: Extract<QuestionForm, { form: 'text' }>): boolean {
    switch (form.echo) {
      case 'shown':
        return false;
      case 'hidden':
        return true;
      default:
        unrecognised(form.echo);
        return true;
    }
  }

  /**
   * `draft` with `option` picked or unpicked. A single choice keeps one
   * answer: picking an option clears the operator's own words.
   */
  static pick(question: Question, draft: Draft, option: string): Draft {
    if (question.form.form !== 'choice' || draft.form !== 'choice') return draft;
    const picked = draft.picked.includes(option);
    if (Questions.single(question.form)) {
      return { form: 'choice', picked: picked ? [] : [option], written: '' };
    }
    return {
      ...draft,
      picked: picked ? draft.picked.filter((label) => label !== option) : [...draft.picked, option],
    };
  }

  /** `draft` with the operator's own words; a single choice drops its pick. */
  static write(question: Question, draft: Draft, written: string): Draft {
    if (question.form.form !== 'choice' || draft.form !== 'choice') return draft;
    return Questions.single(question.form) && written.trim()
      ? { form: 'choice', picked: [], written }
      : { ...draft, written };
  }

  /** The reply a draft makes; a draft with nothing in it skips its question. */
  static reply(question: Question, draft: Draft): WireReplyBody {
    switch (draft.form) {
      case 'choice': {
        const written = draft.written.trim();
        if (draft.picked.length === 0 && !written) return { as: 'skipped' };
        return written
          ? { as: 'chose', picked: draft.picked, written }
          : { as: 'chose', picked: draft.picked };
      }
      case 'text':
        if (!draft.text.trim()) return { as: 'skipped' };
        return question.form.form === 'text' && Questions.hidden(question.form)
          ? { as: 'secret', text: draft.text }
          : { as: 'wrote', text: draft.text };
      case 'number':
        return draft.value === null ? { as: 'skipped' } : { as: 'number', value: draft.value };
      case 'confirm':
        return draft.answer === null
          ? { as: 'skipped' }
          : { as: 'confirmed', answer: draft.answer };
      case 'none':
        return { as: 'skipped' };
      default:
        return assertNever(draft);
    }
  }

  /** The request that sends `drafts`, one per question in order. */
  static request(questions: readonly Question[], drafts: readonly Draft[]): WireReplyRequest {
    const replies: Record<string, WireReplyBody> = {};
    questions.forEach((question, index) => {
      const draft = drafts[index] ?? Questions.draft(question);
      replies[question.key] = Questions.reply(question, draft);
    });
    return { replies };
  }

  /** Whether any question has an answer: replies that skip them all decline instead. */
  static answered(questions: readonly Question[], drafts: readonly Draft[]): boolean {
    return questions.some(
      (question, index) =>
        Questions.reply(question, drafts[index] ?? Questions.draft(question)).as !== 'skipped',
    );
  }

  /** One line naming what an approval of `questions` asks. */
  static summary(questions: readonly Question[]): string {
    const [first] = questions;
    if (!first) return 'Answer questions';
    return questions.length === 1 ? first.text : `${first.text} (and ${questions.length - 1} more)`;
  }
}
