import { REN_GATEWAY_URL } from '$app/env/private';
import { managementResponse } from '#lib/server/management.ts';
import type { RequestHandler } from './$types';

export const GET: RequestHandler = ({ request }) => managementResponse(REN_GATEWAY_URL, request);
export const HEAD = GET;
