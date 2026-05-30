<?php

namespace App\Filament\Resources\Transactions\Schemas;
use App\Models\Service;
use Filament\Forms\Components\DatePicker;
use Filament\Forms\Components\TextInput;
use Filament\Forms\Components\Hidden;
use Filament\Forms\Components\Repeater;
use Filament\Forms\Components\Select;
use Filament\Schemas\Components\Section;
use Filament\Schemas\Components\Utilities\Get;
use Filament\Schemas\Components\Utilities\Set;
use Filament\Schemas\Schema;

class TransactionForm
{
    public static function configure(Schema $schema): Schema
    {
        return $schema
            ->components([
              Section::make('Informasi Transaksi')
                ->columnSpanFull()
                ->schema([  
                Select::make('customer_id')
                        ->relationship('customer', 'name')
                        ->searchable()
                        ->preload()
                        ->required(),
                DatePicker::make('transaction_date')
                    ->required()
                    ->default(now()),
                TextInput::make('total')
                    ->required()
                    ->readOnly()
                    ->numeric()
                    ->dehydrated()
                    ->default(0.0),
                ])->columns(3),
                
                Section::make('Detail Laundry')
                    ->columnSpanFull()
                    ->schema([
                        Repeater::make('details')
                        ->relationship('details')
                        ->live()
                        ->afterStateUpdated(function (Set $set, ?array $state) {
                                $total = collect($state)
                                    ->sum(fn ($item) => (float) ($item['subtotal'] ?? 0));

                                $set('total', $total);
                            })
                        ->schema([
                            Select::make('service_id')
                                ->relationship('service', 'name')
                                ->searchable()
                                ->preload()
                                ->required()
                                ->live()
                                ->afterStateUpdated(
                                    function ($state, Set $set) {

                                        $service = Service::find($state);

                                        if ($service) {
                                            $set('price', $service->price);
                                            $set('subtotal', $service->price);
                                        }
                                    }
                                ),

                                TextInput::make('qty')
                                ->numeric()
                                ->default(1)
                                ->required()
                                ->live()
                                ->afterStateUpdated(
                                    function (Get $get, Set $set) {

                                        $qty = $get('qty') ?? 0;
                                        $price = $get('price') ?? 0;

                                        $set(
                                            'subtotal',
                                            $qty * $price
                                        );
                                    }
                                ),

                            TextInput::make('price')
                                ->numeric()
                                ->readOnly()
                                ->dehydrated(),

                            TextInput::make('subtotal')
                                ->numeric()
                                ->readOnly()
                                ->dehydrated(),
                        ])->columns(4)
                        ->defaultItems(1)
                        ->live(),
                    ])        

            ]);
    }
}
