// Why a control request was refused, by its `error` code: the daemon's
// control API codes (`controlRefusals`, pinned to the Rust `ControlRefusal`
// by tests/labels.test.ts) and the codes this UI server answers with when it
// refuses before the daemon is asked (`proxyRefusals`, server/control.ts).
// Pure; tested in tests/refusal.test.ts.
import { controlRefusals, type ControlRefusal } from './api.ts';
import { isRecord, label, unrecognised, type Stored } from './decode.ts';

/** Codes the UI server's control proxy refuses with on its own. */
export const proxyRefusals = [
  /** No `REN_CONTROL_TOKEN_FILE`: this UI server drives no runs. */
  'runs_off',
  /** A request that did not come from this UI's own pages. */
  'cross_origin',
  /** A Tailscale login `REN_CONTROL_LOGINS` does not list. */
  'login_not_allowed',
  'image_required',
  'json_required',
  'too_large',
  'no_control_token',
  'unexpected_redirect',
  'daemon_unreachable',
] as const;
export type ProxyRefusal = (typeof proxyRefusals)[number];

/** Every refusal code a control request can come back with. */
export type Refusal = ControlRefusal | ProxyRefusal;
export const refusals: readonly Refusal[] = [...controlRefusals, ...proxyRefusals];

const refusal = label(refusals);

/** What each refusal means to the operator. */
export class Refusals {
  /** The `error` code of a refused answer's body, if it has one. */
  static read(body: unknown): Stored<Refusal> | null {
    return isRecord(body) && typeof body.error === 'string' ? refusal(body.error, 'error') : null;
  }

  /** The operator's words for `code`. Every known code has its own case. */
  static say(code: Stored<Refusal>): string {
    switch (code) {
      // Access.
      case 'cross_site':
      case 'unauthorized':
      case 'forbidden':
        return 'The daemon refused the UI server’s control token.';
      case 'cross_origin':
      case 'login_not_allowed':
        return 'This UI does not let you drive runs.';
      case 'runs_off':
        return 'This UI server does not drive runs: set REN_CONTROL_TOKEN_FILE.';
      case 'no_control_token':
        return 'The UI server cannot read the daemon’s control token. Is the daemon running?';
      case 'daemon_unreachable':
        return 'Cannot reach the Ren daemon.';
      case 'unexpected_redirect':
        return 'The daemon answered with a redirect the UI server does not follow.';
      case 'invalid_request':
      case 'json_required':
        return 'The daemon did not take the request as sent.';
      case 'not_found':
        return 'The daemon does not know that.';
      case 'unavailable':
        return 'The daemon is not serving that right now.';
      // Operator controls.
      case 'not_running':
        return 'That call is no longer running.';
      case 'unrecorded':
        return 'The recorder has no room to record this right now.';
      case 'unrecordable':
        return 'This could not be recorded, so it was not done.';
      case 'backup_running':
        return 'A backup is already running.';
      case 'backup_incomplete':
        return 'The backup could not finish; try again.';
      case 'backup_failed':
        return 'The backup failed. The daemon’s log says why.';
      // History.
      case 'call_open':
        return 'A call of it is still open; delete it once the call ends.';
      case 'run_running':
        return 'A run of it is still running; stop it before deleting.';
      case 'unreadable':
        return 'The daemon could not read what this needs.';
      // Starting a session.
      case 'runs_disabled':
        return 'Runs are off on the daemon: add a folder to [runs] roots.';
      case 'invalid_directory':
        return 'That folder is not under one of the daemon’s run roots.';
      case 'directory_not_found':
        return 'That folder does not exist on the daemon’s host.';
      case 'not_drivable':
        return 'This daemon does not know that session’s client.';
      case 'no_approvals':
        return 'That client asks for nothing, so it takes no approval policy.';
      case 'no_scratch':
        return 'The daemon has no scratch folder for a session without one.';
      case 'no_worktree':
        return 'A worktree could not be made there: is the folder a git repository with a commit?';
      case 'worktrees_full':
        return 'Every worktree slot is taken; delete a session that has a worktree first.';
      case 'spawn_failed':
        return 'The client could not be started; is it installed on the daemon’s host?';
      case 'models_unavailable':
        return 'The client could not list its models.';
      case 'commands_unavailable':
        return 'The client could not list its commands.';
      // What the run's state refuses.
      case 'starting':
        return 'The run is still opening its conversation.';
      case 'busy':
        return 'A turn is already running.';
      case 'not_working':
      case 'turn_not_started':
        return 'No turn is running.';
      case 'no_such_approval':
        return 'That approval no longer waits.';
      case 'stopping':
      case 'ended':
        return 'The run is stopping or has ended.';
      case 'nothing_queued':
        return 'Nothing is queued.';
      case 'not_offered':
        return 'That client cannot do this in a running conversation.';
      case 'protocol':
        return 'The client is not in a state to take that now.';
      case 'client_fixed':
        return 'The session has been asked something, so its client stays.';
      case 'folder_fixed':
        return 'The session has been asked something, so its folder stays.';
      case 'archived':
        return 'The session is archived; unarchive it first.';
      case 'not_archived':
        return 'The session is not archived.';
      case 'no_process':
        return 'No client is running for this session.';
      // Attachments.
      case 'attachment_unrecognized':
      case 'image_required':
        return 'Not a PNG, JPEG, GIF, or WebP image.';
      case 'attachment_unreadable':
        return 'The image’s size cannot be read.';
      case 'attachment_too_wide':
        return 'The image is too large on a side.';
      case 'attachment_too_large':
      case 'too_large':
        return 'The image is too large.';
      case 'attachment_empty':
        return 'The file is empty.';
      case 'attachment_name':
        return 'The file needs a name.';
      case 'too_many_attachments':
        return 'Too many images for one prompt.';
      case 'no_images':
        return 'This client does not take images.';
      case 'no_attachments':
        return 'This daemon keeps no attachments.';
      case 'attachment_sent':
        return 'The image went out with a prompt and stays.';
      case 'attachment_storage':
        return 'The daemon could not keep or read the image.';
      case 'blank_prompt':
        return 'The prompt is empty.';
      // Sign-in.
      case 'sign_in_not_offered':
        return 'Ren cannot sign this client in yet.';
      case 'not_installed':
        return 'The client is not installed on the daemon’s host.';
      case 'no_sign_in':
      case 'not_awaiting_answer':
        return 'This sign-in is no longer waiting for that. Start again.';
      case 'redirect_not_an_address':
        return 'Paste the whole address from the browser’s address bar, starting with http://127.0.0.1.';
      case 'redirect_elsewhere':
        return 'That is not the address this sign-in waits on. Copy it from the tab the link opened.';
      case 'redirect_incomplete':
        return 'That address has no code yet: approve on the page first, then copy the address it goes to.';
      // Tools.
      case 'tools_disabled':
        return 'The daemon could not set up its tools. Its log says why.';
      case 'tools_busy':
        return 'Another change to the tools is in progress. Try again when it ends.';
      case 'tools_no_generation':
      case 'tools_collected':
        return 'That generation cannot be restored.';
      case 'tools_unsupported':
        return 'Ren installs tools only on Linux x64, Linux arm64, and macOS arm64.';
      case 'tools_refused':
        return 'The daemon refused the change.';
      case 'tools_failed':
        return 'The daemon could not read or write its tool store. Its log says why.';
      case 'invalid_order':
        return 'A work item needs at least one mark with a note, within its limits.';
      case 'work_state':
        return 'That work item is past this step already.';
      case 'not_promotable':
        return "That is not the canary's build, or it did not pass its checks.";
      case 'invalid_project':
        return 'A project needs a lowercase name, an absolute repository path, and a factory.';
      case 'project_taken':
        return 'Another project already has that name or that repository.';
      case 'project_mismatch':
        return 'That session works for another project than the work item is for.';
      default:
        return `Refused: ${unrecognised(code)}.`;
    }
  }

  /** Whether `code` refuses a pasted sign-in address, so another can be pasted. */
  static redirect(code: Stored<Refusal> | null): boolean {
    return (
      code === 'redirect_not_an_address' ||
      code === 'redirect_elsewhere' ||
      code === 'redirect_incomplete'
    );
  }
}
