import axios from 'axios';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { fastdlApi } from '../api';
import { deferred, settle } from './memory-renderer';

afterEach(() => vi.restoreAllMocks());

describe('game configuration API', () => {
  it('updates files before sending canonical saved commands sequentially through authenticated RCON', async () => {
    const files = deferred<{ data: { configured: boolean; configuration: string[] } }>();
    const firstCommand = deferred<{ data: { output: string } }>();
    const commands = ['sv_downloadurl "https://downloads.example.test/saved/"', 'sv_allowdownload 1'];
    const post = vi.spyOn(axios, 'post').mockReturnValueOnce(files.promise).mockReturnValueOnce(firstCommand.promise).mockResolvedValue({ data: { output: '' } });

    const applying = fastdlApi.applyConfiguration(4);
    expect(post).toHaveBeenCalledExactlyOnceWith('/api/plugins/i3z7ix336msd4/servers/4/fastdl/configure', {});
    files.resolve({ data: { configured: true, configuration: commands } });
    await settle();
    expect(post).toHaveBeenCalledTimes(2);
    expect(post).toHaveBeenNthCalledWith(2, '/api/servers/4/rcon', { command: commands[0] });
    firstCommand.resolve({ data: { output: '' } });
    await expect(applying).resolves.toEqual({ configured: true, rcon_applied: true });
    expect(post).toHaveBeenNthCalledWith(3, '/api/servers/4/rcon', { command: commands[1] });
  });

  it('never sends RCON when configuration file updates fail', async () => {
    const failure = { response: { data: { code: 'GAME_CONFIG_UPDATE_FAILED' } } };
    const post = vi.spyOn(axios, 'post').mockRejectedValue(failure);
    await expect(fastdlApi.applyConfiguration(4)).rejects.toBe(failure);
    expect(post).toHaveBeenCalledTimes(1);
  });

  it('does not report success or send RCON when files were not configured', async () => {
    const post = vi.spyOn(axios, 'post').mockResolvedValue({ data: { configured: false, configuration: ['sv_allowdownload 1'] } });
    await expect(fastdlApi.applyConfiguration(4)).resolves.toEqual({ configured: false, rcon_applied: false });
    expect(post).toHaveBeenCalledTimes(1);
  });

  it.each([400, 403, 412, 422, 503])('preserves file-update success when RCON returns %s', async (status) => {
    const post = vi.spyOn(axios, 'post')
      .mockResolvedValueOnce({ data: { configured: true, configuration: ['sv_downloadurl "http://example.test/"', 'sv_allowdownload 1'] } })
      .mockRejectedValueOnce({ response: { status } });
    await expect(fastdlApi.applyConfiguration(4)).resolves.toEqual({ configured: true, rcon_applied: false });
    expect(post).toHaveBeenCalledTimes(2);
  });
});
