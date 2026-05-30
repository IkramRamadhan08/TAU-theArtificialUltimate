<?php

namespace App\Filament\Resources\Transactions\Schemas;

use Filament\Infolists\Components\TextEntry;
use Filament\Infolists\Components\RepeatableEntry;
use Filament\Schemas\Components\Section;
use Filament\Schemas\Schema;

class TransactionInfolist
{
    public static function configure(Schema $schema): Schema
    {
        return $schema
            ->components([

                Section::make('Informasi Transaksi')
                    ->columnSpanFull()
                    ->schema([

                        TextEntry::make('invoice_no')
                            ->label('Invoice'),

                        TextEntry::make('customer.name')
                            ->label('Customer'),

                        TextEntry::make('transaction_date')
                            ->date(),

                        TextEntry::make('total')
                            ->money('IDR'),

                    ])
                    ->columns(4),

                Section::make('Detail Laundry')
                    ->columnSpanFull()
                    ->schema([

                        RepeatableEntry::make('details')
                            ->schema([

                                TextEntry::make('service.name')
                                    ->label('Service'),

                                TextEntry::make('qty'),

                                TextEntry::make('price')
                                    ->money('IDR'),

                                TextEntry::make('subtotal')
                                    ->money('IDR'),

                            ])
                            ->columns(4),

                    ]),
            ]);
    }
}