import { REN_DEV_TRAFFIC_URL } from '$app/env/private';
import { devTrafficResponse } from '#lib/server/devtraffic.ts';
import type { RequestHandler } from './$types';

export const GET: RequestHandler = ({ request }) =>
  devTrafficResponse(REN_DEV_TRAFFIC_URL, request);
export const POST = GET;
