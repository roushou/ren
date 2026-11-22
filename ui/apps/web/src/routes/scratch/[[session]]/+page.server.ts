import { SessionDeskServer } from '#lib/server/sessionDesk.ts';
import type { PageServerLoad } from './$types';

export const load: PageServerLoad = (event) => SessionDeskServer.load(event);
