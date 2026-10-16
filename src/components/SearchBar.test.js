import { describe, it, expect, vi } from 'vitest';
import { fireEvent, render } from '@testing-library/svelte';
import SearchBar from './SearchBar.svelte';

describe('SearchBar Component', () => {
  it('renders exact search placeholder when not in semantic mode', () => {
    const { getByPlaceholderText } = render(SearchBar, { props: { isOpen: true } });
    expect(getByPlaceholderText('Belgede ara')).toBeTruthy();
  });

  it('input değişikliğini kontrollü query modeliyle yalnız bir kez bildirir', async () => {
    const listener = vi.fn();
    const { getByPlaceholderText } = render(SearchBar, {
      props: { isOpen: true, query: '', search: listener, onsearch: listener },
    });

    await fireEvent.input(getByPlaceholderText('Belgede ara'), {
      target: { value: 'İstanbul & <PDF>' },
    });

    expect(listener).toHaveBeenCalled();
  });

  it('parent query değerini değiştirince inputu aynı kaynaktan temizler', async () => {
    const { rerender, getByPlaceholderText } = render(SearchBar, {
      props: { isOpen: true, query: 'önceki' },
    });
    const input = getByPlaceholderText('Belgede ara');
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
    expect(getByText('Arama hatası')).toBeTruthy();
    expect(queryByText('1 / 3')).toBeNull();
  });

  it('eşleşme konumunu ve boş sonucu sade metinle gösterir', async () => {
    const { getByRole, rerender } = render(SearchBar, {
      props: { isOpen: true, query: 'rapor', totalMatches: 12, currentMatchIndex: 2 },
    });
    expect(getByRole('status').textContent.trim()).toBe('3 / 12');

    await rerender({ isOpen: true, query: 'rapor', totalMatches: 0 });
    expect(getByRole('status').textContent.trim()).toBe('Eşleşme yok');
  });
});
