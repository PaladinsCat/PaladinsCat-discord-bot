import assert from 'node:assert/strict';
import test from 'node:test';
import { championAutocompleteChoices, commandData } from '../src/commands.js';
import type { Champion } from '../src/types.js';

const champions: Champion[] = [
  { id: 1, name: 'Androxus' },
  { id: 2, name: 'Ash' },
  { id: 3, name: 'Mal\'Damba' },
  { id: 4, name: 'Sha Lin' },
  { id: 5, name: 'Yagorath' },
];

test('champion autocomplete shows an alphabetical roster when focused', () => {
  assert.deepEqual(championAutocompleteChoices(champions, ''), [
    { name: 'Androxus', value: 'Androxus' },
    { name: 'Ash', value: 'Ash' },
    { name: 'Mal\'Damba', value: 'Mal\'Damba' },
    { name: 'Sha Lin', value: 'Sha Lin' },
    { name: 'Yagorath', value: 'Yagorath' },
  ]);
});

test('champion autocomplete handles partial words and punctuation-free aliases', () => {
  assert.deepEqual(championAutocompleteChoices(champions, 'damba'), [
    { name: 'Mal\'Damba', value: 'Mal\'Damba' },
  ]);
  assert.deepEqual(championAutocompleteChoices(champions, 'sha'), [
    { name: 'Sha Lin', value: 'Sha Lin' },
  ]);
});

test('champion autocomplete respects Discord\'s 25-choice response limit', () => {
  const roster = Array.from({ length: 59 }, (_, index) => ({
    id: index + 1,
    name: `Champion ${String(index + 1).padStart(2, '0')}`,
  }));
  assert.equal(championAutocompleteChoices(roster, '').length, 25);
});

test('every registered champion option enables Discord autocomplete', () => {
  const championOptions = commandData.flatMap((command) => command.options ?? [])
    .filter((option) => option.name === 'champion');
  assert.ok(championOptions.length > 0);
  assert.ok(championOptions.every((option) => 'autocomplete' in option && option.autocomplete === true));
});

test('registers singular loadout with player and autocompleting champion options', () => {
  const loadout = commandData.find((command) => command.name === 'loadout');
  assert.ok(loadout);
  assert.equal(commandData.some((command) => command.name === 'loadouts'), false);
  assert.deepEqual(loadout.options?.map((option) => option.name), ['player', 'champion']);
  const champion = loadout.options?.find((option) => option.name === 'champion');
  assert.ok(champion && 'autocomplete' in champion && champion.autocomplete === true);
});

test('registers champion with a required lobby choice list led by Global', () => {
  const champion = commandData.find((command) => command.name === 'champion');
  assert.ok(champion);
  assert.deepEqual(champion.options?.map((option) => option.name), ['champion', 'lobby']);
  const lobby = champion.options?.find((option) => option.name === 'lobby');
  assert.ok(lobby && 'required' in lobby && lobby.required === true);
  const choices = 'choices' in lobby ? lobby.choices?.map(({ name, value }) => ({ name, value })) : undefined;
  assert.deepEqual(choices, [
    { name: 'Global ranked lobbies', value: 'global' },
    { name: 'Bronze–Gold lobbies', value: 'bronze-gold' },
    { name: 'Platinum+ lobbies', value: 'platinum' },
    { name: 'Diamond+ lobbies', value: 'diamond' },
  ]);
});

test('does not register retired leaderboard, random, or status commands', () => {
  const names = commandData.map((command) => command.name);
  assert.equal(names.includes('leaderboard'), false);
  assert.equal(names.includes('random'), false);
  assert.equal(names.includes('status'), false);
});

test('registers maps, composition, and tier-filtered items commands', () => {
  const names = commandData.map((command) => command.name);
  assert.ok(names.includes('maps'));
  assert.ok(names.includes('composition'));
  const items = commandData.find((command) => command.name === 'items');
  assert.ok(items);
  assert.deepEqual(items.options?.map((option) => option.name), ['lobby']);
  const lobby = items.options?.[0];
  assert.ok(lobby && 'required' in lobby && lobby.required === true);
  assert.deepEqual('choices' in lobby ? lobby.choices?.map(({ value }) => value) : undefined, [
    'global', 'bronze-gold', 'platinum', 'diamond',
  ]);
});
