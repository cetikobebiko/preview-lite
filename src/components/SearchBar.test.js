import { describe, it, expect, vi } from 'vitest';
import { fireEvent, render } from '@testing-library/svelte';
import SearchBar from './SearchBar.svelte';

describe('SearchBar Component', () => {
  it('renders exact search placeholder when not in semantic mode', () => {
    const { getByPlaceholderText } = render(SearchBar, { props: { isOpen: true } });
    expect(getByPlaceholderText('BELGE İÇİNDE ARA...')).toBeTruthy();
  });

  it('input değişikliğini kontrollü query modeliyle yalnız bir kez bildirir', async () => {
    const listener = vi.fn();
    const { getByPlaceholderText } = render(SearchBar, {
      props: { isOpen: true, query: '', search: listener, onsearch: listener },
    });

    await fireEvent.input(getByPlaceholderText('BELGE İÇİNDE ARA...'), {
      target: { value: 'İstanbul & <PDF>' },
    });

    expect(listener).toHaveBeenCalled();
  });

  it('parent query değerini değiştirince inputu aynı kaynaktan temizler', async () => {
    const { rerender, getByPlaceholderText } = render(SearchBar, {
      props: { isOpen: true, query: 'önceki' },
    });
    const input = getByPlaceholderText('BELGE İÇİNDE ARA...');
    expect(input.value).toBe('önceki');

    await rerender({ isOpen: true, query: '' });
    expect(input.value).toBe('');
  });

  it('IPC hata durumunu eski sonuç sayısı yerine gösterir', () => {
    const { getByText, queryByText } = render(SearchBar, {
      props: {
        isOpen: true,
        query: 'query',
        totalMatches: 3,
        error: 'ipc failed',
      },
    });
    expect(getByText('SEARCH ERROR')).toBeTruthy();
    expect(queryByText('MATCH: 01/03')).toBeNull();
  });
});
